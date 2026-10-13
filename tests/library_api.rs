use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use xczs::{Args, server::Server};

fn private_state_dir() -> assert_fs::TempDir {
    let state_dir = assert_fs::TempDir::new().expect("create temporary state directory");
    std::fs::set_permissions(state_dir.path(), std::fs::Permissions::from_mode(0o700))
        .expect("make state directory private");
    state_dir
}

fn authenticated_args(root: &Path, state_dir: &Path) -> Args {
    let args = Args {
        root: state_dir.to_path_buf(),
        serve_path: root
            .canonicalize()
            .expect("canonicalize temporary shared root"),
        state_dir: Some(state_dir.join("state")),
        ..Args::default()
    };
    if !state_dir.join("state/state.sqlite3").exists()
        && Server::check_reserved_path_conflicts(&args.serve_path).is_ok()
    {
        Server::initialize(args.clone(), "user", "test-password")
            .expect("explicit fixture initialization");
    }
    args
}

#[tokio::test]
async fn reusable_server_layer_can_be_constructed_without_starting_a_process() {
    let root = assert_fs::TempDir::new().expect("create temporary shared root");
    let state_dir = private_state_dir();
    let args = authenticated_args(root.path(), state_dir.path());

    let runtime = Server::builder(args)
        .build()
        .expect("construct reusable server layer");
    runtime.shutdown().await.expect("drain and close state");
}

#[tokio::test]
async fn reusable_server_can_own_an_isolated_list_snapshot_cache() {
    let root = assert_fs::TempDir::new().expect("create temporary shared root");
    let state_dir = private_state_dir();
    let runtime = Server::builder(authenticated_args(root.path(), state_dir.path()))
        .with_isolated_list_snapshot_cache()
        .build()
        .expect("construct server with an isolated listing cache");

    runtime.shutdown().await.expect("drain and close state");
}

#[tokio::test]
async fn runtime_shutdown_is_observable_and_idempotent() {
    let root = assert_fs::TempDir::new().expect("create temporary shared root");
    let state_dir = private_state_dir();
    let runtime = Server::builder(authenticated_args(root.path(), state_dir.path()))
        .build()
        .expect("construct reusable server layer");

    assert!(runtime.active_task_counts().0 > 0);
    runtime.shutdown().await.expect("drain and close state");
    assert_eq!(runtime.active_task_counts(), (0, 0));

    runtime.shutdown().await.expect("drain and close state");
    assert_eq!(runtime.active_task_counts(), (0, 0));
}

#[tokio::test]
async fn explicit_initialization_creates_private_state_database() {
    let root = assert_fs::TempDir::new().expect("create temporary shared root");
    let state_dir = private_state_dir();
    let state_db = state_dir.path().join("state/state.sqlite3");
    let args = authenticated_args(root.path(), state_dir.path());

    let runtime = Server::builder(args)
        .build()
        .expect("construct server with persistent state");
    runtime.shutdown().await.expect("drain and close state");

    let metadata = std::fs::metadata(&state_db).expect("inspect SQLite state database");
    assert!(metadata.is_file());
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    let header = std::fs::read(&state_db).expect("read SQLite state database");
    assert!(header.starts_with(b"SQLite format 3\0"));
}

#[tokio::test]
async fn dropping_runtime_cancels_its_lifecycle() {
    let root = assert_fs::TempDir::new().expect("create temporary shared root");
    let state_dir = private_state_dir();
    let runtime = Server::builder(authenticated_args(root.path(), state_dir.path()))
        .build()
        .expect("construct reusable server layer");
    let shutdown = runtime.shutdown_token();
    let force_shutdown = runtime.force_shutdown_token();

    drop(runtime);

    assert!(shutdown.is_cancelled());
    assert!(force_shutdown.is_cancelled());
}

#[test]
fn builder_without_tokio_runtime_returns_an_error() {
    let root = assert_fs::TempDir::new().expect("create temporary shared root");
    let state_dir = private_state_dir();
    let result = Server::builder(authenticated_args(root.path(), state_dir.path())).build();

    let error = result.err().expect("construction should require Tokio");
    assert!(error.to_string().contains("active Tokio runtime"));
}

fn platform_handle() -> xcss::server_runtime::RuntimeHandle {
    xcss::server_runtime::platform_handle(xcss::server_runtime::ProductDescriptor {
        id: "xczs".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        common_revision: env!("XCSS_REVISION").into(),
        profile: "server-control-plane".into(),
        capabilities: vec!["server-runtime".into()],
    })
    .expect("valid platform descriptor")
}

#[tokio::test]
async fn http_boundary_routes_without_source_address_identification() {
    use tower::ServiceExt;
    let root = assert_fs::TempDir::new().unwrap();
    let state = private_state_dir();
    let runtime = Server::builder(authenticated_args(root.path(), state.path()))
        .build()
        .unwrap();
    let service = xcss::server_runtime::request_service(
        runtime
            .server()
            .clone()
            .http_service(platform_handle())
            .unwrap(),
    );
    let response = service
        .oneshot(
            http::Request::builder()
                .uri("/")
                .header("accept", "text/html")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), http::StatusCode::SEE_OTHER);
    assert!(response.headers().contains_key("x-request-id"));
    assert!(
        response.headers()["location"]
            .to_str()
            .unwrap()
            .starts_with("/__xczs__/login")
    );
    runtime.shutdown().await.unwrap();
}

#[tokio::test]
async fn retained_service_cannot_register_work_after_state_close() {
    use tower::ServiceExt;
    let root = assert_fs::TempDir::new().unwrap();
    let state = private_state_dir();
    let runtime = Server::builder(authenticated_args(root.path(), state.path()))
        .build()
        .unwrap();
    let service = xcss::server_runtime::request_service(
        runtime
            .server()
            .clone()
            .http_service(platform_handle())
            .unwrap(),
    );
    runtime.shutdown().await.unwrap();
    let operation = uuid::Uuid::new_v4().to_string();
    let request = http::Request::builder()
        .method("DELETE")
        .uri("/never-created")
        .header("x-xczs-operation-id", &operation)
        .body(axum::body::Body::empty())
        .unwrap();
    let response = service.oneshot(request).await.unwrap();
    assert_eq!(response.status(), 503);
    assert_eq!(response.headers()["x-xczs-operation-id"], operation);
    assert_eq!(response.headers()["x-xczs-operation-state"], "rejected");
    assert_eq!(runtime.active_task_counts(), (0, 0));
    assert!(!root.path().join("never-created").exists());
}

#[tokio::test]
async fn reserved_root_conflicts_fail_before_state_creation_without_changing_files() {
    for path in ["healthz", "readyz", "api/v1/auth"] {
        let root = assert_fs::TempDir::new().unwrap();
        let state = private_state_dir();
        let conflict = root.path().join(path);
        std::fs::create_dir_all(conflict.parent().unwrap()).unwrap();
        std::fs::write(&conflict, "preserve existing contents").unwrap();
        let error = Server::builder(authenticated_args(root.path(), state.path()))
            .build()
            .err()
            .unwrap();
        assert!(
            error.to_string().contains("reserved platform path"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read_to_string(&conflict).unwrap(),
            "preserve existing contents"
        );
        assert_eq!(std::fs::read_dir(state.path()).unwrap().count(), 0);
    }
}

#[test]
fn platform_conflict_scan_does_not_follow_symlink_ancestors_or_reserve_all_api_files() {
    let root = assert_fs::TempDir::new().unwrap();
    let outside = assert_fs::TempDir::new().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("api")).unwrap();
    assert!(Server::check_reserved_path_conflicts(root.path()).is_err());
    assert!(root.path().join("api").is_symlink());
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    let ordinary = assert_fs::TempDir::new().unwrap();
    std::fs::create_dir(ordinary.path().join("api")).unwrap();
    std::fs::write(ordinary.path().join("api/notes.txt"), "ordinary").unwrap();
    Server::check_reserved_path_conflicts(ordinary.path()).unwrap();
}
