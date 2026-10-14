//! Isolated installed-state fixtures. No Server installation API is used.
#![allow(dead_code)]
use anyhow::Context;
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
    time::Duration,
};
use xczs::{Args, server::Server};
#[path = "administrator.rs"]
mod administrator;
fn private(path: &Path) -> anyhow::Result<()> {
    match fs::create_dir(path) {
        Ok(()) => fs::set_permissions(path, fs::Permissions::from_mode(0o700))?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(path)?;
            anyhow::ensure!(
                metadata.is_dir() && metadata.permissions().mode() & 0o777 == 0o700,
                "existing fixture directory must be private and physical"
            );
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
async fn pool(path: &Path) -> anyhow::Result<SqlitePool> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?
        .sync_all()?;
    Ok(SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(false)
                .journal_mode(SqliteJournalMode::Delete)
                .synchronous(SqliteSynchronous::Extra)
                .foreign_keys(true)
                .busy_timeout(Duration::from_secs(5)),
        )
        .await?)
}
pub fn initialize(args: &Args, username: &str, password: &str) -> anyhow::Result<()> {
    Server::check_reserved_path_conflicts(&args.serve_path)?;
    let state = args
        .state_dir
        .as_ref()
        .context("fixture data_dir required")?;
    anyhow::ensure!(
        args.root.is_absolute() && args.serve_path.is_absolute() && state.is_absolute(),
        "fixture roots must be absolute"
    );
    private(state)?;
    for name in ["logs", "run"] {
        let path = args.root.join(name);
        if !path.exists() {
            private(&path)?;
        }
    }
    let root = fs::metadata(&args.serve_path)?;
    anyhow::ensure!(root.is_dir(), "fixture shared root must exist");
    let device = root.dev().to_be_bytes();
    let inode = root.ino().to_be_bytes();
    let shared = args.serve_path.canonicalize()?;
    let identity = Server::expected_schema_identity()?;
    // Pools require Tokio, unlike the public direct-connection blocking API.
    // A scoped worker owns its runtime so synchronous fixtures also work when
    // the calling test is already inside either kind of Tokio runtime.
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                runtime.block_on(async {
                    let state_pool = pool(&state.join("state.sqlite3")).await?;
                    let result = async {
                        let mut tx = state_pool.begin().await?;
                        sqlx::raw_sql(include_str!(concat!(
                            env!("OUT_DIR"),
                            "/current_schema.sql"
                        )))
                        .execute(&mut *tx)
                        .await?;
                        let actual = xcss::sqlite::schema_fingerprint(&mut *tx).await?;
                        anyhow::ensure!(
                            actual == identity.schema_sha256,
                            "fixture current DDL fingerprint differs"
                        );
                        sqlx::query(xcss::schema_identity::PRODUCT_METADATA_INSERT_SQL)
                            .bind(&identity.application)
                            .bind(&identity.application_version)
                            .bind(i64::try_from(identity.schema_revision)?)
                            .bind(&identity.schema_sha256)
                            .execute(&mut *tx)
                            .await?;
                        sqlx::query(include_str!("../../schema/initialize-shared-root.sql"))
                            .bind(device.as_slice())
                            .bind(inode.as_slice())
                            .execute(&mut *tx)
                            .await?;
                        tx.commit().await?;
                        let service = xcss::admin_core::AdministratorService::new(
                            xcss::admin_sqlite::SqliteAdministratorStore::new(state_pool.clone()),
                        );
                        let now = u64::try_from(
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)?
                                .as_micros(),
                        )?;
                        administrator::seed_administrator(&service, username, password, now)
                            .await?;
                        Ok::<_, anyhow::Error>(())
                    }
                    .await;
                    state_pool.close().await;
                    result?;
                    let tags = pool(&state.join("tags.db")).await?;
                    let result = async {
                        let mut tx = tags.begin().await?;
                        sqlx::raw_sql(include_str!("../../schema/tagging.sql"))
                            .execute(&mut *tx)
                            .await?;
                        sqlx::query(include_str!("../../schema/initialize-tag-root.sql"))
                            .bind(shared.to_str().context("fixture root must be UTF-8")?)
                            .execute(&mut *tx)
                            .await?;
                        tx.commit().await?;
                        Ok::<_, anyhow::Error>(())
                    }
                    .await;
                    tags.close().await;
                    result
                })
            })
            .join()
            .map_err(|_| anyhow::anyhow!("isolated database fixture worker panicked"))?
    })
}
