use super::{
    capacity::{self, Exhausted, Limits},
    pagination::{self, Cursor, PAGE_SIZE},
};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use sqlx::{Connection as _, QueryBuilder, Row as _, Sqlite, SqliteConnection};
use std::{
    path::Path,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

pub const MAX_FILE_TAGS: i64 = 16;

#[derive(Debug, thiserror::Error)]
#[error("a file can have at most 16 tags")]
pub struct TagLimitExceeded;

#[derive(Debug, thiserror::Error)]
#[error("relink requires distinct records, a missing source, and a current target")]
pub struct RelinkConflict;

#[derive(Clone, Debug)]
pub struct Sample {
    pub path: String,
    pub parent: String,
    pub name: String,
    pub dev: i64,
    pub ino: i64,
    pub size: i64,
    pub mtime_ns: i64,
    pub ctime_ns: i64,
}

#[derive(Debug, Serialize)]
pub struct FileRow {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub size: i64,
    pub mtime_ns: i64,
    pub status: String,
    pub tag_ids: Vec<i64>,
    pub tag_ids_has_more: bool,
}

#[derive(Debug, Serialize)]
pub struct TagRow {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub file_count: i64,
}

#[derive(Debug, Serialize)]
pub struct TagLabel {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct ListedTags {
    pub file_id: Option<i64>,
    pub tags: Vec<TagLabel>,
    pub tags_has_more: bool,
}

pub type TagMatches = std::collections::HashMap<String, ([i64; 5], bool)>;

impl Sample {
    pub fn identity(&self) -> [i64; 5] {
        [self.dev, self.ino, self.size, self.mtime_ns, self.ctime_ns]
    }
}

#[derive(Debug, Serialize)]
pub struct Page {
    pub files: Vec<FileRow>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Serialize)]
pub struct Status {
    pub root: String,
    pub available: bool,
    pub last_scan_at: Option<i64>,
    pub last_error: Option<String>,
    pub indexed: i64,
    pub missing: i64,
    pub suspect: i64,
}

pub struct Database {
    connection: Mutex<Option<SqliteConnection>>,
    pub(in crate::server) admission: std::sync::Arc<tokio::sync::Semaphore>,
    path: std::path::PathBuf,
    limits: Limits,
}
#[derive(Debug, thiserror::Error)]
#[error("tag database is busy")]
pub(in crate::server) struct Busy;
#[derive(Debug, Serialize)]
pub struct TagPage {
    pub tags: Vec<TagRow>,
    pub previous_cursor: Option<String>,
    pub next_cursor: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct FolderPage {
    pub folders: Vec<String>,
    pub previous_cursor: Option<String>,
    pub next_cursor: Option<String>,
}
const CURRENT_SCHEMA: &str = include_str!("../../../schema/tagging.sql");
fn connection_limits() -> xcss::sqlite::ConnectionLimits {
    xcss::sqlite::ConnectionLimits::new(2 * 1024 * 1024)
}
async fn connect(path: &Path) -> Result<SqliteConnection> {
    let mut connection = SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(false)
            .busy_timeout(std::time::Duration::from_secs(2)),
    )
    .await?;
    let result = async {
        xcss::sqlite::apply_connection_limits(&mut connection, connection_limits()).await?;
        xcss::sqlite::enable_defensive(&mut connection).await?;
        sqlx::raw_sql("PRAGMA trusted_schema=OFF; PRAGMA mmap_size=0; PRAGMA cache_size=-2048; PRAGMA temp_store=FILE;")
            .execute(&mut connection).await?;
        Ok::<_, anyhow::Error>(())
    }.await;
    if let Err(error) = result {
        let _ = connection.close().await;
        return Err(error);
    }
    Ok(connection)
}
async fn deadline(connection: &mut SqliteConnection, seconds: u64) -> Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    connection
        .lock_handle()
        .await?
        .set_progress_handler(1000, move || std::time::Instant::now() < deadline);
    Ok(())
}
macro_rules! database_operation {
    ($database:expr, $seconds:expr, $connection:ident, $body:block) => {{
        let mut guard = $database.lock()?;
        let $connection = guard.as_mut().expect("owned tag connection");
        xcss::sqlite::block_on_sqlite_connection(async {
            deadline($connection, $seconds).await?;
            $body
        })
    }};
}
mod read;
#[cfg(test)]
mod tests;
mod write;

impl Drop for Database {
    fn drop(&mut self) {
        let guard = self
            .connection
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(connection) = guard.take()
            && xcss::sqlite::block_on_sqlite_connection(connection.close()).is_err()
        {
            log::error!("The tag SQLite worker could not be closed");
        }
    }
}
impl Database {
    pub fn initialize(path: &Path, root: &Path) -> Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.sync_all()?;
        drop(file);
        xcss::sqlite::block_on_sqlite_connection(async {
            let mut connection = connect(path).await?;
            let result = async {
                let mut tx = connection.begin_with("BEGIN IMMEDIATE").await?;
                sqlx::raw_sql(CURRENT_SCHEMA).execute(&mut *tx).await?;
                sqlx::query("INSERT INTO roots(id,path) VALUES('main',?)")
                    .bind(root.to_str().context("root path must be UTF-8")?)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                Ok::<_, anyhow::Error>(())
            }
            .await;
            let closed = connection.close().await;
            result?;
            closed?;
            Ok(())
        })
    }
    pub fn validate_current(path: &Path, root: &Path) -> Result<()> {
        let snapshot = xcss::sqlite::ValidationSnapshot::capture_with_limits(
            path,
            xcss::sqlite::SnapshotLimits {
                max_total_bytes: Limits::PRODUCTION.database_bytes,
                ..Default::default()
            },
        )?;
        xcss::sqlite::block_on_sqlite_connection(async {
            let mut connection = connect(snapshot.database_path()).await?;
            let result = async {
                deadline(&mut connection, 3).await?;
                sqlx::query("PRAGMA query_only=ON")
                    .execute(&mut connection)
                    .await?;
                anyhow::ensure!(
                    xcss::sqlite::schema_fingerprint(&mut connection).await?
                        == env!("XCZS_TAG_SCHEMA_SHA256"),
                    "tag database does not have the exact current structure"
                );
                let roots: Vec<(String, String)> =
                    sqlx::query_as("SELECT id,path FROM roots ORDER BY id LIMIT 2")
                        .fetch_all(&mut connection)
                        .await?;
                anyhow::ensure!(
                    roots
                        == [(
                            "main".to_owned(),
                            root.to_str().context("root path must be UTF-8")?.to_owned()
                        )],
                    "tag database belongs to a different or invalid root"
                );
                let integrity: Vec<String> = sqlx::query_scalar("PRAGMA integrity_check(32)")
                    .fetch_all(&mut connection)
                    .await?;
                anyhow::ensure!(integrity == ["ok"], "tag database integrity check failed");
                let violations: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM pragma_foreign_key_check")
                        .fetch_one(&mut connection)
                        .await?;
                anyhow::ensure!(violations == 0, "tag database foreign key check failed");
                Ok::<_, anyhow::Error>(())
            }
            .await;
            let closed = connection.close().await;
            result?;
            closed?;
            Ok(())
        })
    }
    pub fn open(path: &Path, root: &Path) -> Result<Self> {
        Self::validate_current(path, root)?;
        let connection = xcss::sqlite::block_on_sqlite_connection(async {
            let mut connection = connect(path).await?;
            let result = async {
                sqlx::raw_sql(
                    "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;",
                )
                .execute(&mut connection)
                .await?;
                Ok::<_, anyhow::Error>(())
            }
            .await;
            if let Err(error) = result {
                let _ = connection.close().await;
                return Err(error);
            }
            Ok(connection)
        })?;
        Ok(Self {
            connection: Mutex::new(Some(connection)),
            admission: std::sync::Arc::new(tokio::sync::Semaphore::new(2)),
            path: path.to_owned(),
            limits: Limits::PRODUCTION,
        })
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Option<SqliteConnection>>> {
        self.connection.try_lock().map_err(|_| Busy.into())
    }
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
fn now() -> Result<i64> {
    Ok(i64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
    )?)
}

pub struct FileQuery {
    pub search: String,
    pub directory: String,
    pub scope: String,
    pub status: String,
    pub all_tags: Vec<i64>,
    pub any_tags: Vec<i64>,
    pub exclude_tags: Vec<i64>,
    pub page: u32,
    pub page_size: u32,
}
