use super::*;
use crate::{Args, server::ServerLifecycle};
use std::os::unix::fs::PermissionsExt;
use tower::ServiceExt;

#[tokio::test]
async fn relink_preserves_admission_retry_contract() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let server = Arc::new(
        Server::initialize_with_lifecycle_for_test(
            Args {
                serve_path: root.path().to_owned(),
                root: state.path().to_path_buf(),
                state_dir: Some(state.path().join("state")),
                ..Args::default()
            },
            ServerLifecycle::new(),
        )
        .unwrap(),
    );
    let sample = |path: &str, ino| super::super::db::Sample {
        path: path.into(),
        parent: "".into(),
        name: path.into(),
        dev: 1,
        ino,
        size: 1,
        mtime_ns: 1,
        ctime_ns: 1,
    };
    let db = &server.tagging.db;
    db.scan_complete(vec![sample("old.txt", 1)]).unwrap();
    let tag = db.create_tag("important", None).unwrap();
    db.mutate_tags(&[1], &[tag], true).unwrap();
    db.scan_complete(vec![sample("new.txt", 2)]).unwrap();
    let second_tag = db.create_tag("existing", None).unwrap();
    db.mutate_tags(&[2], &[second_tag], true).unwrap();
    // A SQLite writer lock is an ordinary temporary storage failure.
    use sqlx::Connection as _;
    let mut blocker = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new().filename(state.path().join("state/tags.db")),
    )
    .await
    .unwrap();
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut blocker)
        .await
        .unwrap();
    let app = Router::new()
        .nest("/api/v1/file-tags", routes())
        .with_state(server.clone());
    let (locked_status, locked_headers, locked_body) = post_relink(&app, 2, 1).await;
    assert_eq!(locked_status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(locked_headers.get("retry-after").unwrap(), "1");
    assert_eq!(locked_body["retryable"], true);
    assert_eq!(locked_body["code"], "service_unavailable");
    sqlx::query("ROLLBACK").execute(&mut blocker).await.unwrap();
    // An unexpected transaction failure must not be mislabeled as a conflict.
    sqlx::query("CREATE TRIGGER fail_relink BEFORE INSERT ON file_tags BEGIN SELECT RAISE(ABORT, 'injected transaction failure'); END").execute(&mut blocker).await.unwrap();
    let (failed_status, _, failed_body) = post_relink(&app, 2, 1).await;
    assert_eq!(failed_status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(failed_body["code"], "request_failed");
    sqlx::query("DROP TRIGGER fail_relink")
        .execute(&mut blocker)
        .await
        .unwrap();
    blocker.close().await.unwrap();
    let admission = server.tagging.db.admission.clone();
    let permit = admission
        .clone()
        .acquire_many_owned(admission.available_permits() as u32)
        .await
        .unwrap();
    let app = Router::new()
        .nest("/api/v1/file-tags", routes())
        .with_state(server.clone());
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/v1/file-tags/files/2/relink")
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(r#"{"source_id":1}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    drop(permit);
    let retry = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/v1/file-tags/files/2/relink")
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(r#"{"source_id":1}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(retry.status(), StatusCode::NO_CONTENT);
    let labels = db.listed_tags(&[Some(sample("new.txt", 2))]).unwrap();
    let mut tags = labels[0].tags.iter().map(|tag| tag.id).collect::<Vec<_>>();
    tags.sort();
    assert_eq!(tags, vec![tag, second_tag]);
    for (target, source) in [(2, 2), (2, 1)] {
        let (status, _, body) = post_relink(&app, target, source).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["code"], "conflict");
    }
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(headers.get("retry-after").unwrap(), "1");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["retryable"],
        true
    );
}

async fn post_relink(
    app: &Router,
    target: i64,
    source: i64,
) -> (StatusCode, axum::http::HeaderMap, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("/api/v1/file-tags/files/{target}/relink"))
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(format!(
                    r#"{{"source_id":{source}}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, headers, body)
}

#[test]
fn relink_error_classes_remain_distinct() {
    for (error, status, code, retryable) in [
        (
            anyhow::Error::new(super::super::db::RelinkConflict),
            StatusCode::CONFLICT,
            "conflict",
            false,
        ),
        (
            anyhow::Error::new(super::super::db::TagLimitExceeded),
            StatusCode::CONFLICT,
            "tag_limit_exceeded",
            false,
        ),
        (
            anyhow::Error::new(super::super::db::Busy),
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_requests",
            true,
        ),
        (
            anyhow::anyhow!("unclassified storage failure"),
            StatusCode::INTERNAL_SERVER_ERROR,
            "request_failed",
            false,
        ),
    ] {
        let actual = database_error(error);
        assert_eq!((actual.0, actual.1, actual.2), (status, code, retryable));
    }
}

#[tokio::test]
async fn deleted_tag_identity_is_not_reused() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let server = Arc::new(
        Server::initialize_with_lifecycle_for_test(
            Args {
                serve_path: root.path().to_owned(),
                root: state.path().to_path_buf(),
                state_dir: Some(state.path().join("state")),
                ..Args::default()
            },
            ServerLifecycle::new(),
        )
        .unwrap(),
    );

    let db = &server.tagging.db;
    let sample = super::super::db::Sample {
        path: "notes.txt".into(),
        parent: "".into(),
        name: "notes.txt".into(),
        dev: 1,
        ino: 1,
        size: 1,
        mtime_ns: 1,
        ctime_ns: 1,
    };
    db.scan_complete(vec![sample]).unwrap();
    let app = Router::new()
        .nest("/api/v1/file-tags", routes())
        .with_state(server.clone());
    async fn request(
        app: &Router,
        method: &str,
        path: &str,
        body: &str,
    ) -> (StatusCode, serde_json::Value) {
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method(method)
                    .uri(format!("/api/v1/file-tags{path}"))
                    .header("Content-Type", "application/json")
                    .body(axum::body::Body::from(body.to_owned()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let value = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        println!("{method} {path} body={body} -> {status}: {value}");
        (status, value)
    }
    let (status, old) = request(&app, "POST", "/tags", r#"{"name":"work"}"#).await;
    assert_eq!(status, StatusCode::CREATED);
    let stale_id = old["id"].as_i64().unwrap();
    request(&app, "GET", "/tags", "").await;
    assert_eq!(
        request(&app, "DELETE", &format!("/tags/{stale_id}"), "")
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let replacement = request(&app, "POST", "/tags", r#"{"name":"personal"}"#)
        .await
        .1;
    let posted = format!(r#"{{"file_ids":[1],"tag_ids":[{stale_id}],"action":"add"}}"#);
    let (mutation_status, _) = request(&app, "POST", "/file-tags", &posted).await;
    let tags = request(&app, "GET", "/files/1/tags", "").await.1;
    println!(
        "Cached UI label=work, replacement id={replacement}, actual returned file tag labels={tags}, stale mutation status={mutation_status}"
    );
    assert_ne!(
        replacement["id"], old["id"],
        "deleted tag identity must never be reused"
    );
    assert_eq!(mutation_status, StatusCode::NOT_FOUND);
    assert_eq!(tags["tags"], json!([]));
    let new_id = replacement["id"].as_i64().unwrap();
    let assign_current = format!(r#"{{"file_ids":[1],"tag_ids":[{new_id}],"action":"add"}}"#);
    assert_eq!(
        request(&app, "POST", "/file-tags", &assign_current).await.0,
        StatusCode::NO_CONTENT
    );
    let remove_stale = format!(r#"{{"file_ids":[1],"tag_ids":[{stale_id}],"action":"remove"}}"#);
    assert_eq!(
        request(&app, "POST", "/file-tags", &remove_stale).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app,
            "PUT",
            &format!("/tags/{stale_id}"),
            r#"{"name":"stale edit"}"#
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, "DELETE", &format!("/tags/{stale_id}"), "")
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let after = request(&app, "GET", "/files/1/tags", "").await.1;
    assert_eq!(after["tags"].as_array().unwrap().len(), 1);
    assert_eq!(after["tags"][0]["id"], new_id);
    assert_eq!(after["tags"][0]["name"], "personal");
}
