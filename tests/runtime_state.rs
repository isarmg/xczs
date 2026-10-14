#[path = "support/database.rs"]
mod fixture_database;
use std::{fs, os::unix::fs::PermissionsExt};
use xcss::admin_core::LoginContext;
use xczs::{Args, server::Server};

fn fixture() -> (tempfile::TempDir, tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("config")).unwrap();
    fs::set_permissions(
        root.path().join("config"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let config = root.path().join("config/config.json");
    fs::write(&config, serde_json::json!({"serve_path":files.path(),"bind":"127.0.0.1:0","development":true,"min_free_space":0}).to_string()).unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    (root, files, config)
}

#[tokio::test]
async fn persistent_sessions_survive_reopening_and_authenticated_rotation_revokes_them() {
    let (root, _files, config) = fixture();
    let args = Args::load(root.path(), &config).unwrap();
    fixture_database::initialize(&args, "admin", "initial-password").unwrap();
    assert!(!root.path().join("state/administrator").exists());
    let args = Args::load(root.path(), &config).unwrap();
    let pool = Server::administrator_pool(&args).await.unwrap();
    let service = xcss::admin_core::AdministratorService::new(
        xcss::admin_sqlite::SqliteAdministratorStore::new(pool.clone()),
    );
    let context = LoginContext {
        request_id: None,
        now_micros: now(),
    };
    let login = service
        .login("admin", "initial-password", &context)
        .await
        .unwrap();
    pool.close().await;
    let pool = Server::administrator_pool(&args).await.unwrap();
    let service = xcss::admin_core::AdministratorService::new(
        xcss::admin_sqlite::SqliteAdministratorStore::new(pool.clone()),
    );
    assert!(
        service
            .authenticate_session(&login.session_token, now())
            .await
            .is_ok()
    );
    let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&pool)
        .await
        .unwrap();
    let sync: i64 = sqlx::query_scalar("PRAGMA synchronous")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(mode, "delete");
    assert_eq!(sync, 3);
    let identity = service
        .authenticate_session(&login.session_token, now())
        .await
        .unwrap();
    service
        .update_own_account(
            &xcss::admin_core::AdministratorManagementContext {
                identity,
                request_id: None,
                now_micros: now(),
            },
            "admin",
            "initial-password",
            Some("updated-password"),
        )
        .await
        .unwrap();
    pool.close().await;
    let pool = Server::administrator_pool(&args).await.unwrap();
    let service = xcss::admin_core::AdministratorService::new(
        xcss::admin_sqlite::SqliteAdministratorStore::new(pool.clone()),
    );
    assert!(
        service
            .authenticate_session(&login.session_token, now())
            .await
            .is_err()
    );
    pool.close().await;
}

#[tokio::test]
async fn unsupported_schema_and_retired_config_are_rejected_without_writing_business_data() {
    let (root, files, config) = fixture();
    let args = Args::load(root.path(), &config).unwrap();
    fixture_database::initialize(&args, "admin", "initial-password").unwrap();
    fs::write(files.path().join("keep.txt"), b"business data").unwrap();
    let state = root.path().join("state");
    let before = fs::read(state.join("state.sqlite3")).unwrap();
    let tags = fs::read(state.join("tags.db")).unwrap();
    Server::validate_current(&Args::load(root.path(), &config).unwrap()).unwrap();
    assert_eq!(before, fs::read(state.join("state.sqlite3")).unwrap());
    assert_eq!(tags, fs::read(state.join("tags.db")).unwrap());
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(state.join("state.sqlite3"));
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::query("UPDATE product_metadata SET schema_revision=schema_revision+1 WHERE singleton=1")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let unsupported = fs::read(state.join("state.sqlite3")).unwrap();
    assert!(Server::validate_current(&Args::load(root.path(), &config).unwrap()).is_err());
    assert_eq!(unsupported, fs::read(state.join("state.sqlite3")).unwrap());
    assert_eq!(tags, fs::read(state.join("tags.db")).unwrap());
    assert_eq!(
        fs::read(files.path().join("keep.txt")).unwrap(),
        b"business data"
    );
    let mut document: serde_json::Value =
        serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    document["trusted_proxies"] = serde_json::json!(["127.0.0.1/32"]);
    fs::write(&config, serde_json::to_vec(&document).unwrap()).unwrap();
    let old_config = fs::read(&config).unwrap();
    assert!(Args::load(root.path(), &config).is_err());
    assert_eq!(old_config, fs::read(config).unwrap());
    assert_eq!(unsupported, fs::read(state.join("state.sqlite3")).unwrap());
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as u64
}

#[tokio::test]
async fn shared_directory_cannot_expose_control_or_private_namespaces() {
    let (root, _files, config) = fixture();
    for name in [
        "bin",
        "libexec",
        "share",
        "maintenance",
        "config",
        "logs",
        "run",
    ] {
        fs::write(&config, serde_json::json!({"serve_path": root.path().join(name), "bind": "127.0.0.1:0", "development": true}).to_string()).unwrap();
        assert!(
            Args::load(root.path(), &config).is_err(),
            "namespace {name}"
        );
    }
    assert!(!root.path().join("state").exists());
}
