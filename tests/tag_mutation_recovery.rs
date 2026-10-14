#[path = "support/fixtures.rs"]
mod fixtures;

use fixtures::{Error, TestServer, preflight_upload_target, server, with_new_upload_headers};
use reqwest::Method;
use rstest::rstest;
use serde_json::{Value, json};

fn read(server: &TestServer, suffix: &str) -> Result<Value, Error> {
    Ok(serde_json::from_str(
        &server
            .get(server.url().join(&format!("api/v1/file-tags/{suffix}"))?)?
            .text()?,
    )?)
}

fn scan(server: &TestServer) -> Result<(), Error> {
    let response = server
        .request(Method::POST, server.url().join("api/v1/file-tags/scan")?)
        .send()?;
    assert_eq!(response.status(), 200);
    Ok(())
}

fn post(
    server: &TestServer,
    suffix: &str,
    body: Value,
) -> Result<reqwest::blocking::Response, Error> {
    Ok(server
        .request(
            Method::POST,
            server.url().join(&format!("api/v1/file-tags/{suffix}"))?,
        )
        .header("Content-Type", "application/json")
        .body(serde_json::to_vec(&body)?)
        .send()?)
}

#[rstest]
fn deleted_tagged_file_recreated_by_upload_requires_explicit_relink(
    server: TestServer,
) -> Result<(), Error> {
    let name = "tag-recovery.txt";
    let url = server.url().join(name)?;
    let upload = |bytes: &'static [u8]| -> Result<(), Error> {
        let response =
            with_new_upload_headers(server.request(Method::PUT, url.clone()), bytes.len() as u64)
                .body(bytes.to_vec())
                .send()?;
        assert_eq!(response.status(), 201);
        Ok(())
    };
    upload(b"old")?;
    scan(&server)?;
    let old = read(&server, &format!("file?path={name}"))?["file_id"]
        .as_i64()
        .ok_or("old identity missing")?;
    let tag: Value =
        serde_json::from_str(&post(&server, "tags", json!({"name":"Keep me"}))?.text()?)?;
    let tag = tag["id"].as_i64().ok_or("tag missing")?;
    assert_eq!(
        post(
            &server,
            "file-tags",
            json!({"file_ids":[old],"tag_ids":[tag],"action":"add"})
        )?
        .status(),
        204
    );
    let revision = preflight_upload_target(&server, &format!("/{name}"))?
        .revision
        .ok_or("revision missing")?;
    assert_eq!(
        server
            .request(Method::DELETE, url.clone())
            .header("If-Match", format!("\"{revision}\""))
            .send()?
            .status(),
        204
    );
    scan(&server)?;
    let history = read(&server, "files?scope=all&status=missing")?;
    assert!(
        history["files"]
            .as_array()
            .ok_or("history missing")?
            .iter()
            .any(|row| row["id"] == old && row["tag_ids"] == json!([tag]))
    );
    upload(b"new")?;
    assert!(read(&server, &format!("file?path={name}"))?["file_id"].is_null());
    scan(&server)?;
    let current = read(&server, &format!("file?path={name}"))?;
    let new = current["file_id"].as_i64().ok_or("new identity missing")?;
    assert_ne!(new, old);
    assert_eq!(current["tags"], json!([]));
    assert_eq!(
        post(
            &server,
            &format!("files/{new}/relink"),
            json!({"source_id":old})
        )?
        .status(),
        204
    );
    assert_eq!(
        read(&server, &format!("file?path={name}"))?["tags"][0]["id"],
        tag
    );
    assert_eq!(server.get(url)?.bytes()?.as_ref(), b"new");
    Ok(())
}

#[rstest]
fn partial_overwrite_preserves_live_tags_until_resumed_publication(
    server: TestServer,
) -> Result<(), Error> {
    use fixtures::{with_resume_upload_headers, with_upload_overwrite_headers};
    let name = "tagged-overwrite.txt";
    let url = server.url().join(name)?;
    std::fs::write(server.path().join(name), b"original")?;
    scan(&server)?;
    let old = read(&server, &format!("file?path={name}"))?["file_id"]
        .as_i64()
        .ok_or("old identity missing")?;
    let tag: Value =
        serde_json::from_str(&post(&server, "tags", json!({"name":"Before overwrite"}))?.text()?)?;
    let tag = tag["id"].as_i64().ok_or("tag missing")?;
    assert_eq!(
        post(
            &server,
            "file-tags",
            json!({"file_ids":[old],"tag_ids":[tag],"action":"add"})
        )?
        .status(),
        204
    );
    let revision = preflight_upload_target(&server, &format!("/{name}"))?
        .revision
        .ok_or("revision missing")?;
    let upload_id = uuid::Uuid::new_v4();
    let partial = with_upload_overwrite_headers(
        server.request(Method::PUT, url.clone()),
        upload_id,
        6,
        &revision,
    )
    .body(b"abc".to_vec())
    .send()?;
    assert_eq!(partial.status(), 409);
    assert_eq!(partial.headers().get("x-xczs-upload-offset").unwrap(), "3");
    assert_eq!(server.get(url.clone())?.bytes()?.as_ref(), b"original");
    assert_eq!(
        read(&server, &format!("file?path={name}"))?["tags"][0]["id"],
        tag
    );
    scan(&server)?;
    assert_eq!(read(&server, &format!("file?path={name}"))?["file_id"], old);
    let complete =
        with_resume_upload_headers(server.request(Method::PATCH, url.clone()), upload_id, 6, 3)
            .header("X-Xczs-Upload-Overwrite", "true")
            .header("X-Xczs-Target-Revision", revision)
            .body(b"def".to_vec())
            .send()?;
    assert_eq!(complete.status(), 204);
    assert_eq!(server.get(url)?.bytes()?.as_ref(), b"abcdef");
    assert!(read(&server, &format!("file?path={name}"))?["file_id"].is_null());
    scan(&server)?;
    let current = read(&server, &format!("file?path={name}"))?;
    assert_ne!(current["file_id"], old);
    assert_eq!(current["tags"], json!([]));
    let history = read(&server, "files?scope=all&status=missing")?;
    assert!(
        history["files"]
            .as_array()
            .ok_or("history missing")?
            .iter()
            .any(|row| row["id"] == old && row["tag_ids"] == json!([tag]))
    );
    Ok(())
}

#[rstest]
fn stale_tag_edit_reports_missing_records_and_current_state_conflicts(
    server: TestServer,
) -> Result<(), Error> {
    std::fs::write(server.path().join("stale-edit.txt"), b"kept")?;
    scan(&server)?;
    let id = read(&server, "file?path=stale-edit.txt")?["file_id"]
        .as_i64()
        .ok_or("file id missing")?;
    let tag: Value =
        serde_json::from_str(&post(&server, "tags", json!({"name":"Stale edit"}))?.text()?)?;
    let tag = tag["id"].as_i64().ok_or("tag missing")?;
    let edit = |file, tag| {
        post(
            &server,
            "file-tags",
            json!({"file_ids":[file],"tag_ids":[tag],"action":"add"}),
        )
    };
    let missing_file = edit(i64::MAX, tag)?;
    assert_eq!(missing_file.status(), 404);
    assert_eq!(
        serde_json::from_str::<Value>(&missing_file.text()?)?["code"],
        "not_found"
    );
    assert_eq!(edit(id, i64::MAX)?.status(), 404);
    let deleted: Value =
        serde_json::from_str(&post(&server, "tags", json!({"name":"Deleted meanwhile"}))?.text()?)?;
    let deleted = deleted["id"].as_i64().ok_or("tag missing")?;
    assert_eq!(
        server
            .request(
                Method::DELETE,
                server
                    .url()
                    .join(&format!("api/v1/file-tags/tags/{deleted}"))?
            )
            .send()?
            .status(),
        204
    );
    assert_eq!(edit(id, deleted)?.status(), 404);
    // Validation of every ID precedes insertion, so stale mixed batches stay atomic.
    assert_eq!(
        post(
            &server,
            "file-tags",
            json!({"file_ids":[id],"tag_ids":[tag,deleted],"action":"add"})
        )?
        .status(),
        404
    );
    assert_eq!(
        read(&server, "file?path=stale-edit.txt")?["tags"],
        json!([])
    );
    std::fs::remove_file(server.path().join("stale-edit.txt"))?;
    scan(&server)?;
    let conflict = edit(id, tag)?;
    assert_eq!(conflict.status(), 409);
    assert_eq!(
        serde_json::from_str::<Value>(&conflict.text()?)?["code"],
        "conflict"
    );
    let history = read(&server, "files?scope=all&status=missing")?;
    assert!(
        history["files"]
            .as_array()
            .ok_or("history missing")?
            .iter()
            .any(|row| row["id"] == id && row["tag_ids"] == json!([]))
    );
    Ok(())
}

#[rstest]
fn duplicate_tag_names_report_conflict_without_changing_existing_tags(
    server: TestServer,
) -> Result<(), Error> {
    let first = post(&server, "tags", json!({"name":"Unique tag"}))?;
    assert_eq!(first.status(), 201);
    let first: Value = serde_json::from_str(&first.text()?)?;
    let duplicate = post(&server, "tags", json!({"name":"Unique tag"}))?;
    assert_eq!(duplicate.status(), 409);
    assert_eq!(
        serde_json::from_str::<Value>(&duplicate.text()?)?["code"],
        "conflict"
    );
    assert_eq!(
        post(&server, "tags", json!({"name":"unique TAG"}))?.status(),
        409
    );
    let second: Value =
        serde_json::from_str(&post(&server, "tags", json!({"name":"Other tag"}))?.text()?)?;
    let second = second["id"].as_i64().ok_or("second tag missing")?;
    let rename = server
        .request(
            Method::PUT,
            server
                .url()
                .join(&format!("api/v1/file-tags/tags/{second}"))?,
        )
        .header("Content-Type", "application/json")
        .body(r#"{"name":"Unique tag"}"#)
        .send()?;
    assert_eq!(rename.status(), 409);
    assert_eq!(
        serde_json::from_str::<Value>(&rename.text()?)?["code"],
        "conflict"
    );
    let tags = read(&server, "tags")?;
    let tags = tags["tags"].as_array().ok_or("tags missing")?;
    assert!(
        tags.iter()
            .any(|tag| tag["id"] == first["id"] && tag["name"] == "Unique tag")
    );
    assert!(
        tags.iter()
            .any(|tag| tag["id"] == second && tag["name"] == "Other tag")
    );
    Ok(())
}
