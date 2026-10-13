use super::*;

use std::{
    fs::{self, File, OpenOptions, Permissions},
    io::ErrorKind,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
};
use xcss::schema_identity::SchemaIdentity;

#[cfg(test)]
use std::io::{self, Read, Seek, SeekFrom};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
// Repository row counts and Linux root-relative path limits keep legitimate
// state well below this value. Refuse a corrupt or misplaced multi-gigabyte
// file before duplicating it into the system temporary filesystem.
pub(super) const SQLITE_SIDECAR_SUFFIXES: [&str; 3] = ["-journal", "-wal", "-shm"];

pub(super) const CURRENT_SCHEMA: &str =
    include_str!(concat!(env!("OUT_DIR"), "/current_schema.sql"));

#[derive(Eq, PartialEq)]
struct SchemaSnapshot {
    objects: Vec<SchemaObjectSnapshot>,
    tables: Vec<TableSchemaSnapshot>,
}

#[derive(Eq, PartialEq)]
struct SchemaObjectSnapshot {
    object_type: String,
    name: String,
    table_name: String,
    sql: Option<String>,
}

#[derive(Eq, PartialEq)]
struct TableSchemaSnapshot {
    name: String,
    column_count: i64,
    without_rowid: i64,
    strict: i64,
    columns: Vec<ColumnSchemaSnapshot>,
    indexes: Vec<IndexSchemaSnapshot>,
    foreign_key_count: i64,
}

#[derive(Eq, Ord, PartialEq, PartialOrd)]
struct ColumnSchemaSnapshot {
    name: String,
    declared_type: String,
    not_null: i64,
    default_value: Option<String>,
    primary_key_position: i64,
    hidden: i64,
}

#[derive(Eq, Ord, PartialEq, PartialOrd)]
struct IndexSchemaSnapshot {
    name: String,
    unique: i64,
    origin: String,
    partial: i64,
    key_columns: Vec<IndexColumnSchemaSnapshot>,
}

#[derive(Eq, Ord, PartialEq, PartialOrd)]
struct IndexColumnSchemaSnapshot {
    position: i64,
    name: Option<String>,
    descending: i64,
    collation: Option<String>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct SidecarMetadataSnapshot {
    device: u64,
    inode: u64,
    mode: u32,
    links: u64,
    uid: u32,
    gid: u32,
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl SidecarMetadataSnapshot {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            links: metadata.nlink(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            size: metadata.size(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        }
    }
}

/// A generation change is separate from an anchored file's security identity.
/// Read-only diagnostics defer that classification until every guard is checked.
fn compare_guard_metadata(
    expected: SidecarMetadataSnapshot,
    actual: SidecarMetadataSnapshot,
    allow_generation_change: bool,
    diagnostic: &str,
) -> Result<bool> {
    ensure!(
        expected.device == actual.device
            && expected.inode == actual.inode
            && expected.mode == actual.mode
            && expected.links == actual.links
            && expected.uid == actual.uid
            && expected.gid == actual.gid,
        "{diagnostic}"
    );
    let changed = actual != expected;
    ensure!(allow_generation_change || !changed, "{diagnostic}");
    Ok(changed)
}

fn require_private_sidecar_security(
    path: &Path,
    metadata: &fs::Metadata,
    database: SidecarMetadataSnapshot,
) -> Result<()> {
    validate_sidecar_metadata(path, metadata)?;
    ensure!(
        metadata.mode() == database.mode
            && metadata.uid() == database.uid
            && metadata.gid() == database.gid,
        "SQLite sidecar `{}` does not match the private database ownership and permissions",
        path.display()
    );
    Ok(())
}

struct MainDatabaseGuard {
    path: PathBuf,
    file: File,
    snapshot: SidecarMetadataSnapshot,
}

impl MainDatabaseGuard {
    fn inspect(path: &Path) -> Result<Self> {
        Ok(Self::inspect_with_generation_policy(path, false)?.0)
    }

    fn inspect_read_only(path: &Path) -> Result<(Self, bool)> {
        Self::inspect_with_generation_policy(path, true)
    }

    fn inspect_with_generation_policy(
        path: &Path,
        allow_generation_change: bool,
    ) -> Result<(Self, bool)> {
        let path_metadata = fs::symlink_metadata(path)
            .with_context(|| format!("Failed to inspect state database `{}`", path.display()))?;
        validate_main_database_metadata(path, &path_metadata)?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(
                (rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::NOFOLLOW).bits() as i32,
            )
            .open(path)
            .with_context(|| {
                format!("Failed to safely open state database `{}`", path.display())
            })?;
        let opened_metadata = file.metadata().with_context(|| {
            format!(
                "Failed to inspect opened state database `{}`",
                path.display()
            )
        })?;
        validate_main_database_metadata(path, &opened_metadata)?;
        let snapshot = SidecarMetadataSnapshot::from_metadata(&opened_metadata);
        let changed = compare_guard_metadata(
            SidecarMetadataSnapshot::from_metadata(&path_metadata),
            snapshot,
            allow_generation_change,
            &format!(
                "State database `{}` was replaced while it was being inspected",
                path.display()
            ),
        )?;
        Ok((
            Self {
                path: path.to_path_buf(),
                file,
                snapshot,
            },
            changed,
        ))
    }

    fn revalidate(&self) -> Result<()> {
        self.revalidate_with_generation_policy(false).map(|_| ())
    }

    fn revalidate_read_only(&self) -> Result<bool> {
        self.revalidate_with_generation_policy(true)
    }

    fn revalidate_with_generation_policy(&self, allow_generation_change: bool) -> Result<bool> {
        let opened_metadata = self.file.metadata().with_context(|| {
            format!(
                "Failed to re-inspect opened state database `{}`",
                self.path.display()
            )
        })?;
        validate_main_database_metadata(&self.path, &opened_metadata)?;
        let mut changed = compare_guard_metadata(
            self.snapshot,
            SidecarMetadataSnapshot::from_metadata(&opened_metadata),
            allow_generation_change,
            &format!(
                "Opened state database `{}` changed identity or metadata",
                self.path.display()
            ),
        )?;
        let path_metadata = fs::symlink_metadata(&self.path).with_context(|| {
            format!(
                "State database `{}` disappeared or was replaced after validation",
                self.path.display()
            )
        })?;
        validate_main_database_metadata(&self.path, &path_metadata)?;
        changed |= compare_guard_metadata(
            self.snapshot,
            SidecarMetadataSnapshot::from_metadata(&path_metadata),
            allow_generation_change,
            &format!(
                "State database `{}` was replaced after validation",
                self.path.display()
            ),
        )?;
        Ok(changed)
    }
}

struct SqliteSidecarGuard {
    entries: Vec<SqliteSidecarEntry>,
}

struct SqliteSidecarEntry {
    path: PathBuf,
    state: SqliteSidecarState,
}

enum SqliteSidecarState {
    Absent,
    Present {
        file: File,
        snapshot: SidecarMetadataSnapshot,
    },
}

impl SqliteSidecarGuard {
    fn inspect(database_path: &Path) -> Result<Self> {
        Ok(Self::inspect_with_security(database_path, None)?.0)
    }

    fn inspect_read_only(
        database_path: &Path,
        security: SidecarMetadataSnapshot,
    ) -> Result<(Self, bool)> {
        Self::inspect_with_security(database_path, Some(security))
    }

    fn inspect_with_security(
        database_path: &Path,
        security: Option<SidecarMetadataSnapshot>,
    ) -> Result<(Self, bool)> {
        let mut entries = Vec::with_capacity(SQLITE_SIDECAR_SUFFIXES.len());
        let mut changed = false;
        for suffix in SQLITE_SIDECAR_SUFFIXES {
            let path = sqlite_sidecar_path(database_path, suffix);
            let state = match fs::symlink_metadata(&path) {
                Ok(metadata) => {
                    validate_sidecar_metadata(&path, &metadata)?;
                    let file = OpenOptions::new()
                        .read(true)
                        .custom_flags(
                            (rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::NOFOLLOW).bits()
                                as i32,
                        )
                        .open(&path)
                        .with_context(|| {
                            format!("Failed to safely open SQLite sidecar `{}`", path.display())
                        })?;
                    let opened_metadata = file.metadata().with_context(|| {
                        format!(
                            "Failed to inspect opened SQLite sidecar `{}`",
                            path.display()
                        )
                    })?;
                    validate_sidecar_metadata(&path, &opened_metadata)?;
                    let expected = SidecarMetadataSnapshot::from_metadata(&metadata);
                    let opened = SidecarMetadataSnapshot::from_metadata(&opened_metadata);
                    changed |= compare_guard_metadata(
                        expected,
                        opened,
                        security.is_some(),
                        &format!(
                            "SQLite sidecar `{}` was replaced while it was being inspected",
                            path.display()
                        ),
                    )?;
                    SqliteSidecarState::Present {
                        file,
                        snapshot: opened,
                    }
                }
                Err(error) if error.kind() == ErrorKind::NotFound => SqliteSidecarState::Absent,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("Failed to inspect SQLite sidecar `{}`", path.display())
                    });
                }
            };
            entries.push(SqliteSidecarEntry { path, state });
        }
        let guard = Self { entries };
        changed |= guard.revalidate_with_security(security)?;
        Ok((guard, changed))
    }

    fn revalidate(&self) -> Result<()> {
        self.revalidate_with_security(None).map(|_| ())
    }

    fn revalidate_read_only(&self, security: SidecarMetadataSnapshot) -> Result<bool> {
        self.revalidate_with_security(Some(security))
    }

    fn revalidate_with_security(&self, security: Option<SidecarMetadataSnapshot>) -> Result<bool> {
        let mut changed = false;
        for entry in &self.entries {
            match &entry.state {
                SqliteSidecarState::Absent => match fs::symlink_metadata(&entry.path) {
                    Err(error) if error.kind() == ErrorKind::NotFound => {}
                    Ok(metadata) => {
                        let Some(security) = security else {
                            bail!(
                                "SQLite sidecar `{}` appeared or was replaced after validation",
                                entry.path.display()
                            );
                        };
                        require_private_sidecar_security(&entry.path, &metadata, security)?;
                        let file = OpenOptions::new()
                            .read(true)
                            .custom_flags(
                                (rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::NOFOLLOW).bits()
                                    as i32,
                            )
                            .open(&entry.path)?;
                        let opened_metadata = file.metadata()?;
                        require_private_sidecar_security(&entry.path, &opened_metadata, security)?;
                        let named_metadata = fs::symlink_metadata(&entry.path)?;
                        require_private_sidecar_security(&entry.path, &named_metadata, security)?;
                        let opened = SidecarMetadataSnapshot::from_metadata(&opened_metadata);
                        compare_guard_metadata(
                            SidecarMetadataSnapshot::from_metadata(&metadata),
                            opened,
                            true,
                            &format!(
                                "SQLite sidecar `{}` changed identity or security metadata while appearing",
                                entry.path.display()
                            ),
                        )?;
                        compare_guard_metadata(
                            opened,
                            SidecarMetadataSnapshot::from_metadata(&named_metadata),
                            true,
                            &format!(
                                "SQLite sidecar `{}` was replaced while appearing",
                                entry.path.display()
                            ),
                        )?;
                        changed = true;
                    }
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!(
                                "Failed to re-inspect absent SQLite sidecar `{}`",
                                entry.path.display()
                            )
                        });
                    }
                },
                SqliteSidecarState::Present { file, snapshot } => {
                    let opened_metadata = file.metadata().with_context(|| {
                        format!(
                            "Failed to re-inspect opened SQLite sidecar `{}`",
                            entry.path.display()
                        )
                    })?;
                    let named = fs::symlink_metadata(&entry.path);
                    if security.is_some()
                        && named
                            .as_ref()
                            .is_err_and(|error| error.kind() == ErrorKind::NotFound)
                    {
                        // A real unlink drops the anchored inode's last link. A rename
                        // or replacement retains/changes identity and stays a hard failure.
                        let mut unlinked = SidecarMetadataSnapshot::from_metadata(&opened_metadata);
                        ensure!(
                            opened_metadata.is_file() && snapshot.links == 1 && unlinked.links == 0,
                            "SQLite sidecar `{}` disappeared without a normal unlink",
                            entry.path.display()
                        );
                        unlinked.links = snapshot.links;
                        compare_guard_metadata(
                            *snapshot,
                            unlinked,
                            true,
                            &format!(
                                "Opened SQLite sidecar `{}` changed identity or security metadata during removal",
                                entry.path.display()
                            ),
                        )?;
                        changed = true;
                        continue;
                    }
                    validate_sidecar_metadata(&entry.path, &opened_metadata)?;
                    changed |= compare_guard_metadata(
                        *snapshot,
                        SidecarMetadataSnapshot::from_metadata(&opened_metadata),
                        security.is_some(),
                        &format!(
                            "Opened SQLite sidecar `{}` changed identity or security metadata",
                            entry.path.display()
                        ),
                    )?;
                    let path_metadata = named.with_context(|| {
                        format!(
                            "SQLite sidecar `{}` disappeared or was replaced after validation",
                            entry.path.display()
                        )
                    })?;
                    validate_sidecar_metadata(&entry.path, &path_metadata)?;
                    changed |= compare_guard_metadata(
                        *snapshot,
                        SidecarMetadataSnapshot::from_metadata(&path_metadata),
                        security.is_some(),
                        &format!(
                            "SQLite sidecar `{}` was replaced after validation",
                            entry.path.display()
                        ),
                    )?;
                }
            }
        }
        Ok(changed)
    }

    fn has_present_sidecar(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| matches!(&entry.state, SqliteSidecarState::Present { .. }))
    }
}

fn sqlite_sidecar_path(database_path: &Path, suffix: &str) -> PathBuf {
    let mut path = database_path.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

fn validate_sidecar_metadata(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    ensure!(
        !metadata.file_type().is_symlink(),
        "SQLite sidecar `{}` cannot be a symbolic link",
        path.display()
    );
    ensure!(
        metadata.file_type().is_file(),
        "SQLite sidecar `{}` must be a regular file",
        path.display()
    );
    ensure!(
        metadata.nlink() == 1,
        "SQLite sidecar `{}` cannot have multiple hard links",
        path.display()
    );
    Ok(())
}

fn validate_main_database_metadata(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    ensure!(
        !metadata.file_type().is_symlink(),
        "State database `{}` cannot be a symbolic link",
        path.display()
    );
    ensure!(
        metadata.file_type().is_file(),
        "State database `{}` must be a regular file",
        path.display()
    );
    ensure!(
        metadata.nlink() == 1,
        "State database `{}` cannot have multiple hard links",
        path.display()
    );
    Ok(())
}

pub(super) fn prepare_database_file(path: &Path, root: &RootIdentity) -> Result<()> {
    ensure!(
        path.file_name().is_some(),
        "State database path must name a file"
    );
    // Reject an unsafe pre-existing SQLite sidecar before creating, chmodding,
    // or opening the main database. Holding the safe fds also keeps their
    // inspected identities anchored until this filesystem preparation ends.
    let sidecars = SqliteSidecarGuard::inspect(path)?;
    if sidecars.has_present_sidecar() {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == ErrorKind::NotFound => bail!(
                "Refusing to create state database `{}` while a SQLite sidecar already exists",
                path.display()
            ),
            Ok(_) => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Failed to inspect state database `{}`", path.display())
                });
            }
        }
    }
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent_metadata = fs::metadata(parent).with_context(|| {
        format!(
            "Failed to inspect state database directory `{}`",
            parent.display()
        )
    })?;
    ensure!(
        parent_metadata.is_dir(),
        "State database parent `{}` is not a directory",
        parent.display()
    );

    match OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
    {
        Ok(file) => {
            file.set_permissions(Permissions::from_mode(0o600))
                .with_context(|| {
                    format!(
                        "Failed to set state database permissions on `{}`",
                        path.display()
                    )
                })?;
            file.sync_all().with_context(|| {
                format!(
                    "Failed to synchronize new state database `{}`",
                    path.display()
                )
            })?;
            fs::File::open(parent)
                .and_then(|directory| directory.sync_all())
                .with_context(|| {
                    format!(
                        "Failed to synchronize state database directory `{}`",
                        parent.display()
                    )
                })?;
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(path).with_context(|| {
                format!("Failed to inspect state database `{}`", path.display())
            })?;
            ensure!(
                !metadata.file_type().is_symlink(),
                "State database `{}` cannot be a symbolic link",
                path.display()
            );
            ensure!(
                metadata.is_file(),
                "State database `{}` is not a regular file",
                path.display()
            );
            ensure!(
                metadata.nlink() == 1,
                "State database `{}` cannot have multiple hard links",
                path.display()
            );

            // Validate before changing permissions or any persistent SQLite
            // setting so an unrelated file is never silently adopted.
            preflight_existing_database(path, *root)?;
            fs::set_permissions(path, Permissions::from_mode(0o600)).with_context(|| {
                format!(
                    "Failed to set state database permissions on `{}`",
                    path.display()
                )
            })?;
        }
        Err(err) => {
            return Err(err)
                .with_context(|| format!("Failed to create state database `{}`", path.display()));
        }
    }
    Ok(())
}

pub(super) fn open_initialized_connection(
    path: &Path,
    root: RootIdentity,
    operation_ttl_ms: i64,
    upload_ttl_ms: i64,
    recovery_now_ms: i64,
    temporary_root: &Path,
    require_administrator: bool,
) -> Result<Connection> {
    let mut connection = open_connection_with_sidecar_guard(
        path,
        root,
        temporary_root,
        || Ok(()),
        require_administrator,
    )?;
    let result = xcss::sqlite::block_on_sqlite_connection(async {
        validate_product_metadata(&mut connection).await?;
        verify_root_identity(&mut connection, root).await?;
        configure_validated_connection(&mut connection).await?;
        recover_database(
            &mut connection,
            operation_ttl_ms,
            upload_ttl_ms,
            recovery_now_ms,
        )
        .await
    });
    if let Err(error) = result {
        let _ = xcss::sqlite::block_on_sqlite_connection(connection.close());
        return Err(error);
    }
    Ok(connection)
}

pub(in crate::server) fn initialize_current(path: &Path, root: RootIdentity) -> Result<()> {
    ensure!(
        !path.try_exists()?,
        "state database already exists; initialization never overwrites data"
    );
    prepare_database_file(path, &root)?;
    xcss::sqlite::block_on_sqlite_connection(async {
        let mut connection = connect_direct(path).await?;
        let result = async {
            harden_connection(&mut connection).await?;
            validate_database_before_mutation(&mut connection, root).await?;
            configure_validated_connection(&mut connection).await?;
            initialize_schema(&mut connection, root).await?;
            sqlx::raw_sql("PRAGMA wal_checkpoint(TRUNCATE)")
                .execute(&mut connection)
                .await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        let closed = connection.close().await;
        result?;
        closed?;
        Ok(())
    })
}

#[cfg(test)]
pub(in crate::server) fn validate_current(path: &Path, root: RootIdentity) -> Result<()> {
    validate_current_in(path, root, path.parent().context("test state parent")?)
}
#[cfg(test)]
pub(in crate::server) fn validate_current_in(
    path: &Path,
    root: RootIdentity,
    temporary_root: &Path,
) -> Result<()> {
    let mut connection =
        open_existing_database_after_snapshot(path, root, true, temporary_root, || Ok(()))?;
    xcss::sqlite::block_on_sqlite_connection(async {
        validate_product_metadata(&mut connection).await?;
        verify_root_identity(&mut connection, root).await?;
        validate_integrity(&mut connection).await?;
        Ok(())
    })
}

pub(in crate::server) fn validate_current_with_administrator_in(
    path: &Path,
    root: RootIdentity,
    temporary_root: &Path,
) -> Result<()> {
    let mut connection =
        open_existing_database_after_snapshot(path, root, true, temporary_root, || Ok(()))?;
    xcss::sqlite::block_on_sqlite_connection(async {
        xcss::admin_sqlite::validate_administrators_on_connection(&mut connection).await?;
        Ok(())
    })
}

async fn connect_direct(path: &Path) -> Result<Connection> {
    Ok(Connection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(false)
            .busy_timeout(BUSY_TIMEOUT),
    )
    .await?)
}

/// Own the captured generation until its native worker has explicitly closed.
pub(super) struct ValidationConnection {
    connection: Option<Connection>,
    _snapshot: xcss::sqlite::ValidationSnapshot,
}
impl std::fmt::Debug for ValidationConnection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ValidationConnection")
            .finish_non_exhaustive()
    }
}
impl std::ops::Deref for ValidationConnection {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.connection
            .as_ref()
            .expect("owned validation connection")
    }
}
impl std::ops::DerefMut for ValidationConnection {
    fn deref_mut(&mut self) -> &mut Connection {
        self.connection
            .as_mut()
            .expect("owned validation connection")
    }
}
impl Drop for ValidationConnection {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take()
            && xcss::sqlite::block_on_sqlite_connection(connection.close()).is_err()
        {
            log::error!("The private SQLite validation worker could not be closed");
        }
    }
}

fn open_existing_database_for_preflight(
    path: &Path,
    root: RootIdentity,
) -> Result<ValidationConnection> {
    open_existing_database_for_preflight_after_snapshot(path, root, || Ok(()))
}

fn open_connection_with_sidecar_guard<F>(
    path: &Path,
    root: RootIdentity,
    temporary_root: &Path,
    after_snapshot: F,
    require_administrator: bool,
) -> Result<Connection>
where
    F: FnOnce() -> Result<()>,
{
    let sidecars = SqliteSidecarGuard::inspect(path)?;
    let main_database = MainDatabaseGuard::inspect(path)?;
    after_snapshot()?;
    main_database.revalidate()?;
    sidecars.revalidate()?;
    let expected = main_database.snapshot;
    let snapshot = xcss::sqlite::ValidationSnapshot::capture_in(path, temporary_root)?;
    validate_raw_main_snapshot(&snapshot, root)?;
    xcss::sqlite::block_on_sqlite_connection(async {
        let mut private = connect_direct(snapshot.database_path()).await?;
        let result = async {
            harden_connection(&mut private).await?;
            validate_database_before_mutation(&mut private, root).await?;
            if require_administrator {
                xcss::admin_sqlite::validate_administrators_on_connection(&mut private).await?;
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;
        let closed = private.close().await;
        result?;
        closed?;
        Ok::<_, anyhow::Error>(())
    })?;
    drop(snapshot);
    main_database.revalidate()?;
    sidecars.revalidate()?;
    // Finish and drop raw descriptors before opening the original SQLite generation.
    drop(main_database);
    drop(sidecars);
    xcss::sqlite::block_on_sqlite_connection(async {
        let mut connection = connect_direct(path).await?;
        let result = async {
            let metadata = fs::symlink_metadata(path)?;
            ensure!(
                metadata.dev() == expected.device
                    && metadata.ino() == expected.inode
                    && metadata.is_file()
                    && !metadata.file_type().is_symlink(),
                "State database identity changed while opening SQLite"
            );
            harden_connection(&mut connection).await
        }
        .await;
        if let Err(error) = result {
            let _ = connection.close().await;
            return Err(error);
        }
        Ok(connection)
    })
}

fn open_existing_database_for_preflight_after_snapshot<F>(
    path: &Path,
    root: RootIdentity,
    after_snapshot: F,
) -> Result<ValidationConnection>
where
    F: FnOnce() -> Result<()>,
{
    open_existing_database_after_snapshot(
        path,
        root,
        false,
        path.parent().context("state parent")?,
        after_snapshot,
    )
}

fn open_existing_database_after_snapshot<F>(
    path: &Path,
    root: RootIdentity,
    read_only_diagnostics: bool,
    temporary_root: &Path,
    after_snapshot: F,
) -> Result<ValidationConnection>
where
    F: FnOnce() -> Result<()>,
{
    let (main_database, sidecars) = if read_only_diagnostics {
        let (main_database, main_changed) = MainDatabaseGuard::inspect_read_only(path)?;
        let (sidecars, sidecars_changed) =
            SqliteSidecarGuard::inspect_read_only(path, main_database.snapshot)?;
        // Check every anchored identity before classifying any generation change.
        let main_rechecked = main_database.revalidate_read_only()?;
        let sidecars_rechecked = sidecars.revalidate_read_only(main_database.snapshot)?;
        if main_changed || sidecars_changed || main_rechecked || sidecars_rechecked {
            return Err(xcss::sqlite::SnapshotError::SourceChanged.into());
        }
        (main_database, sidecars)
    } else {
        // Initialization and runtime preflight retain their original strict guards.
        let sidecars = SqliteSidecarGuard::inspect(path)?;
        let main_database = MainDatabaseGuard::inspect(path)?;
        (main_database, sidecars)
    };
    let snapshot = xcss::sqlite::ValidationSnapshot::capture_in(path, temporary_root)?;
    after_snapshot()?;
    if read_only_diagnostics {
        let main_changed = main_database.revalidate_read_only()?;
        let sidecars_changed = sidecars.revalidate_read_only(main_database.snapshot)?;
        if main_changed || sidecars_changed {
            return Err(xcss::sqlite::SnapshotError::SourceChanged.into());
        }
    } else {
        main_database.revalidate()?;
        sidecars.revalidate()?;
    }
    validate_raw_main_snapshot(&snapshot, root)?;
    drop(main_database);
    drop(sidecars);
    let connection = xcss::sqlite::block_on_sqlite_connection(async {
        let mut connection = connect_direct(snapshot.database_path()).await?;
        let result = async {
            harden_connection(&mut connection).await?;
            validate_database_before_mutation(&mut connection, root).await
        }
        .await;
        if let Err(error) = result {
            let _ = connection.close().await;
            return Err(error);
        }
        Ok::<_, anyhow::Error>(connection)
    })?;
    Ok(ValidationConnection {
        connection: Some(connection),
        _snapshot: snapshot,
    })
}

/// Check the captured main separately so a foreign main cannot be hidden by its WAL.
fn validate_raw_main_snapshot(
    snapshot: &xcss::sqlite::ValidationSnapshot,
    root: RootIdentity,
) -> Result<()> {
    let raw_path = snapshot.database_path().with_file_name("raw-main.sqlite3");
    fs::copy(snapshot.database_path(), &raw_path)?;
    xcss::sqlite::block_on_sqlite_connection(async {
        let mut raw = connect_direct(&raw_path).await?;
        let result = async {
            harden_connection(&mut raw).await?;
            validate_database_before_mutation(&mut raw, root).await
                .context("The raw main state database does not have the current schema and root identity")
        }.await;
        let closed = raw.close().await;
        result?;
        closed?;
        Ok(())
    })
}

#[cfg(test)]
fn copy_raw_main_database_snapshot(
    main_database: &MainDatabaseGuard,
    destination: &mut File,
    max_bytes: u64,
) -> Result<()> {
    ensure!(
        main_database.snapshot.size <= max_bytes,
        "State database `{}` is {} bytes and exceeds the raw validation snapshot limit of {} bytes",
        main_database.path.display(),
        main_database.snapshot.size,
        max_bytes
    );
    let copy_limit = max_bytes
        .checked_add(1)
        .ok_or_else(|| anyhow!("Raw state database snapshot limit cannot be incremented"))?;
    let mut source = main_database
        .file
        .try_clone()
        .context("Failed to duplicate the state database validation handle")?;
    source
        .seek(SeekFrom::Start(0))
        .context("Failed to rewind the state database validation handle")?;
    let mut bounded = source.take(copy_limit);
    let copied = io::copy(&mut bounded, destination)
        .context("Failed to copy the raw state database validation snapshot")?;
    ensure!(
        copied <= max_bytes,
        "State database `{}` grew beyond the raw validation snapshot limit of {} bytes while it was being copied",
        main_database.path.display(),
        max_bytes
    );
    ensure!(
        copied == main_database.snapshot.size,
        "State database `{}` changed size while its raw validation snapshot was being copied",
        main_database.path.display()
    );
    Ok(())
}

#[cfg(test)]
pub(super) fn copy_raw_main_database_snapshot_after_inspect_for_test<F>(
    source_path: &Path,
    destination: &mut File,
    max_bytes: u64,
    after_inspect: F,
) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    let main_database = MainDatabaseGuard::inspect(source_path)?;
    after_inspect()?;
    copy_raw_main_database_snapshot(&main_database, destination, max_bytes)
}

#[cfg(test)]
pub(super) fn open_existing_database_after_sidecar_snapshot_for_test<F>(
    path: &Path,
    root: RootIdentity,
    after_snapshot: F,
) -> Result<ValidationConnection>
where
    F: FnOnce() -> Result<()>,
{
    open_existing_database_for_preflight_after_snapshot(path, root, after_snapshot)
}

#[cfg(test)]
pub(super) fn open_read_only_database_after_sidecar_snapshot_for_test<F>(
    path: &Path,
    root: RootIdentity,
    after_snapshot: F,
) -> Result<ValidationConnection>
where
    F: FnOnce() -> Result<()>,
{
    open_existing_database_after_snapshot(
        path,
        root,
        true,
        path.parent().context("test state parent")?,
        after_snapshot,
    )
}

fn connection_limits() -> xcss::sqlite::ConnectionLimits {
    xcss::sqlite::ConnectionLimits::new(2 * 1024 * 1024)
}

pub(in crate::server) async fn harden_connection(connection: &mut Connection) -> Result<()> {
    xcss::sqlite::apply_connection_limits(connection, connection_limits()).await?;
    xcss::sqlite::enable_defensive(connection).await?;
    sqlx::raw_sql("PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON; PRAGMA mmap_size=0; PRAGMA temp_store=MEMORY;")
        .execute(connection).await?;
    Ok(())
}

pub(in crate::server) async fn configure_validated_connection(
    connection: &mut Connection,
) -> Result<()> {
    let mode: String = sqlx::query_scalar("PRAGMA journal_mode=DELETE")
        .fetch_one(&mut *connection)
        .await?;
    ensure!(
        mode.eq_ignore_ascii_case("delete"),
        "SQLite refused rollback DELETE journal mode"
    );
    sqlx::query("PRAGMA synchronous=EXTRA")
        .execute(&mut *connection)
        .await?;
    let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
        .fetch_one(&mut *connection)
        .await?;
    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&mut *connection)
        .await?;
    let trusted_schema: i64 = sqlx::query_scalar("PRAGMA trusted_schema")
        .fetch_one(&mut *connection)
        .await?;
    let mmap_size: i64 = sqlx::query_scalar("PRAGMA mmap_size")
        .fetch_one(&mut *connection)
        .await?;
    ensure!(synchronous == 3, "SQLite synchronous mode is not EXTRA");
    ensure!(
        foreign_keys == 1,
        "SQLite foreign key enforcement is disabled"
    );
    ensure!(trusted_schema == 0, "SQLite trusted schema mode is enabled");
    ensure!(mmap_size == 0, "SQLite memory-mapped I/O is enabled");
    Ok(())
}

fn preflight_existing_database(path: &Path, root: RootIdentity) -> Result<()> {
    let mut connection = open_existing_database_for_preflight(path, root)?;
    xcss::sqlite::block_on_sqlite_connection(validate_database_before_mutation(
        &mut connection,
        root,
    ))
    .context("Refusing to modify an unrecognized state database")
}

async fn validate_exact_schema(connection: &mut Connection) -> Result<()> {
    let actual = inspect_schema(connection)
        .await
        .context("Failed to inspect state database schema")?;
    let expected = expected_schema()
        .await
        .context("Failed to construct the current state schema")?;
    ensure!(
        actual == expected,
        "State database does not exactly match the current XCZS schema (tables, columns, constraints, indexes, triggers, or views differ)"
    );
    Ok(())
}

async fn expected_schema() -> Result<SchemaSnapshot> {
    let mut connection = Connection::connect("sqlite::memory:").await?;
    let result = async {
        xcss::sqlite::apply_connection_limits(&mut connection, connection_limits()).await?;
        sqlx::raw_sql(CURRENT_SCHEMA)
            .execute(&mut connection)
            .await?;
        inspect_schema(&mut connection).await
    }
    .await;
    let closed = connection.close().await;
    let schema = result?;
    closed?;
    Ok(schema)
}

async fn inspect_schema(connection: &mut Connection) -> Result<SchemaSnapshot> {
    // The shared reader rejects all excess objects/bytes rather than truncating evidence.
    let rows = xcss::sqlite::schema_rows(&mut *connection).await?;
    let objects = rows
        .into_iter()
        .map(|row| {
            Ok(SchemaObjectSnapshot {
                object_type: row.object_type,
                name: row.name,
                table_name: row.table_name,
                sql: Some(canonical_schema_sql(&row.sql)?),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut names = objects
        .iter()
        .filter(|object| object.object_type == "table")
        .map(|object| object.name.clone())
        .collect::<Vec<_>>();
    names.sort_unstable();
    let mut tables = Vec::with_capacity(names.len());
    for name in names {
        tables.push(inspect_table_schema(connection, name).await?);
    }
    Ok(SchemaSnapshot { objects, tables })
}

async fn inspect_table_schema(
    connection: &mut Connection,
    name: String,
) -> Result<TableSchemaSnapshot> {
    let (column_count, without_rowid, strict): (i64, i64, i64) = sqlx::query_as(
        "SELECT ncol, wr, strict FROM pragma_table_list WHERE schema='main' AND name=? AND type='table'",
    ).bind(&name).fetch_one(&mut *connection).await?;
    let mut columns = sqlx::query(
        "SELECT name,type,\"notnull\",dflt_value,pk,hidden FROM pragma_table_xinfo(?) LIMIT 1025",
    )
    .bind(&name)
    .try_map(|row| {
        Ok(ColumnSchemaSnapshot {
            name: row.try_get(0)?,
            declared_type: row.try_get::<String, _>(1)?.to_ascii_uppercase(),
            not_null: row.try_get(2)?,
            default_value: row.try_get(3)?,
            primary_key_position: row.try_get(4)?,
            hidden: row.try_get(5)?,
        })
    })
    .fetch_all(&mut *connection)
    .await?;
    ensure!(columns.len() <= 1024, "State schema has too many columns");
    for column in &mut columns {
        column.default_value = column
            .default_value
            .take()
            .map(|sql| canonical_schema_sql(&sql))
            .transpose()?;
    }
    columns.sort_unstable();
    let index_rows: Vec<(String, i64, String, i64)> = sqlx::query_as(
        "SELECT name,\"unique\",origin,partial FROM pragma_index_list(?) LIMIT 1025",
    )
    .bind(&name)
    .fetch_all(&mut *connection)
    .await?;
    ensure!(
        index_rows.len() <= 1024,
        "State schema has too many indexes"
    );
    let mut indexes = Vec::with_capacity(index_rows.len());
    for (index_name, unique, origin, partial) in index_rows {
        let key_columns = sqlx::query(
            "SELECT seqno,name,\"desc\",coll FROM pragma_index_xinfo(?) WHERE key=1 ORDER BY seqno LIMIT 1025",
        ).bind(&index_name).try_map(|row| Ok(IndexColumnSchemaSnapshot {
            position: row.try_get(0)?, name: row.try_get(1)?, descending: row.try_get(2)?, collation: row.try_get(3)?,
        })).fetch_all(&mut *connection).await?;
        ensure!(
            key_columns.len() <= 1024,
            "State index has too many columns"
        );
        indexes.push(IndexSchemaSnapshot {
            name: index_name,
            unique,
            origin,
            partial,
            key_columns,
        });
    }
    indexes.sort_unstable();
    let foreign_key_count = sqlx::query_scalar("SELECT COUNT(*) FROM pragma_foreign_key_list(?)")
        .bind(&name)
        .fetch_one(&mut *connection)
        .await?;
    Ok(TableSchemaSnapshot {
        name,
        column_count,
        without_rowid,
        strict,
        columns,
        indexes,
        foreign_key_count,
    })
}

fn canonical_schema_sql(sql: &str) -> Result<String> {
    let mut compact = String::with_capacity(sql.len());
    let mut characters = sql.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            character if character.is_ascii_whitespace() => {}
            '\'' => {
                compact.push(character);
                let mut terminated = false;
                while let Some(quoted) = characters.next() {
                    compact.push(quoted);
                    if quoted == '\'' {
                        if characters.peek() == Some(&'\'') {
                            if let Some(escaped) = characters.next() {
                                compact.push(escaped);
                            }
                        } else {
                            terminated = true;
                            break;
                        }
                    }
                }
                ensure!(
                    terminated,
                    "SQLite schema contains an unterminated string literal"
                );
            }
            '"' => {
                let mut terminated = false;
                while let Some(quoted) = characters.next() {
                    if quoted == '"' {
                        if characters.peek() == Some(&'"') {
                            compact.push('"');
                            characters.next();
                        } else {
                            terminated = true;
                            break;
                        }
                    } else {
                        compact.push(quoted.to_ascii_lowercase());
                    }
                }
                ensure!(
                    terminated,
                    "SQLite schema contains an unterminated identifier"
                );
            }
            _ => compact.push(character.to_ascii_lowercase()),
        }
    }
    while compact.ends_with(';') {
        compact.pop();
    }
    if compact.starts_with("createtable") {
        canonical_create_table_sql(compact)
    } else {
        Ok(compact)
    }
}

fn canonical_create_table_sql(compact: String) -> Result<String> {
    ensure!(
        compact.is_ascii(),
        "SQLite project table definitions must use ASCII syntax"
    );
    let bytes = compact.as_bytes();
    let open = bytes
        .iter()
        .position(|byte| *byte == b'(')
        .ok_or_else(|| anyhow!("SQLite table definition is missing its column list"))?;
    let mut depth = 0_u32;
    let mut in_string = false;
    let mut close = None;
    let mut index = open;
    while index < bytes.len() {
        match bytes[index] {
            b'\'' => {
                if in_string && bytes.get(index + 1) == Some(&b'\'') {
                    index += 1;
                } else {
                    in_string = !in_string;
                }
            }
            b'(' if !in_string => depth += 1,
            b')' if !in_string => {
                ensure!(
                    depth > 0,
                    "SQLite table definition has unbalanced parentheses"
                );
                depth -= 1;
                if depth == 0 {
                    close = Some(index);
                    break;
                }
            }
            _ => {}
        }
        index += 1;
    }
    ensure!(
        !in_string,
        "SQLite table definition has an unterminated string"
    );
    let close =
        close.ok_or_else(|| anyhow!("SQLite table definition has unbalanced parentheses"))?;

    let body = &compact[open + 1..close];
    let mut clauses = Vec::new();
    let mut clause_start = 0;
    depth = 0;
    in_string = false;
    let body_bytes = body.as_bytes();
    let mut body_index = 0;
    while body_index < body_bytes.len() {
        match body_bytes[body_index] {
            b'\'' => {
                if in_string && body_bytes.get(body_index + 1) == Some(&b'\'') {
                    body_index += 1;
                } else {
                    in_string = !in_string;
                }
            }
            b'(' if !in_string => depth += 1,
            b')' if !in_string => {
                ensure!(depth > 0, "SQLite table clause has unbalanced parentheses");
                depth -= 1;
            }
            b',' if !in_string && depth == 0 => {
                clauses.push(&body[clause_start..body_index]);
                clause_start = body_index + 1;
            }
            _ => {}
        }
        body_index += 1;
    }
    ensure!(
        !in_string && depth == 0,
        "SQLite table clause has invalid quoting or parentheses"
    );
    clauses.push(&body[clause_start..]);
    ensure!(
        clauses.iter().all(|clause| !clause.is_empty()),
        "SQLite table definition contains an empty clause"
    );
    clauses.sort_unstable();

    Ok(format!(
        "{}({}){}",
        &compact[..open],
        clauses.join(","),
        &compact[close + 1..]
    ))
}

async fn validate_database_before_mutation(
    connection: &mut Connection,
    root: RootIdentity,
) -> Result<()> {
    if persistent_object_count(connection).await? == 0 {
        return validate_integrity(connection).await;
    }

    validate_product_metadata(connection).await?;
    verify_root_identity(connection, root).await?;
    validate_integrity(connection).await
}

async fn persistent_object_count(connection: &mut Connection) -> Result<i64> {
    sqlx::query(
        "SELECT COUNT(*) FROM sqlite_schema \
             WHERE type IN ('table', 'index', 'trigger', 'view') \
               AND name NOT GLOB 'sqlite_*'",
    )
    .try_map(|row: sqlx::sqlite::SqliteRow| row.try_get(0))
    .fetch_one(&mut *connection)
    .await
    .context("Failed to count state database schema objects")
}

async fn validate_integrity(connection: &mut Connection) -> Result<()> {
    let quick_check: String = sqlx::query("PRAGMA quick_check(1)")
        .try_map(|row: sqlx::sqlite::SqliteRow| row.try_get(0))
        .fetch_one(&mut *connection)
        .await
        .context("Failed to check state database integrity")?;
    ensure!(
        quick_check.eq_ignore_ascii_case("ok"),
        "State database integrity check failed: {quick_check}"
    );
    Ok(())
}

async fn initialize_schema(connection: &mut Connection, root: RootIdentity) -> Result<()> {
    if persistent_object_count(connection).await? != 0 {
        validate_product_metadata(connection).await?;
        verify_root_identity(connection, root).await?;
        return Ok(());
    }

    let expected_fingerprint = expected_schema_fingerprint()?;
    let mut transaction = connection.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::raw_sql(CURRENT_SCHEMA)
        .execute(&mut *transaction)
        .await?;
    let actual_fingerprint = schema_fingerprint(&mut transaction).await?;
    ensure!(
        actual_fingerprint == expected_fingerprint,
        "The compiled state schema fingerprint is inconsistent"
    );
    sqlx::query(
        "INSERT INTO product_metadata(
             singleton, application, application_version, schema_revision, schema_sha256
         ) VALUES (1, ?1, ?2, ?3, ?4)",
    )
    .bind(APPLICATION)
    .bind("1.0.0")
    .bind(CURRENT_SCHEMA_REVISION)
    .bind(expected_fingerprint)
    .execute(&mut *transaction)
    .await
    .map(|result| result.rows_affected())?;
    insert_root_identity(&mut transaction, root).await?;
    validate_product_metadata(&mut transaction).await?;
    verify_root_identity(&mut transaction, root).await?;
    transaction.commit().await?;
    Ok(())
}

pub(super) async fn validate_product_metadata(connection: &mut Connection) -> Result<()> {
    validate_exact_schema(connection).await?;
    xcss::sqlite::require_current_schema(connection, &expected_schema_identity()?)
        .await
        .context("State database metadata or schema fingerprint is not exactly current")?;
    Ok(())
}

pub(super) fn expected_schema_fingerprint() -> Result<String> {
    Ok(env!("XCZS_CURRENT_SCHEMA_SHA256").to_owned())
}

async fn schema_fingerprint(connection: &mut Connection) -> Result<String> {
    Ok(xcss::sqlite::schema_fingerprint(connection).await?)
}

pub(in crate::server) fn expected_schema_identity() -> Result<SchemaIdentity> {
    SchemaIdentity::new(
        APPLICATION,
        "1.0.0",
        u64::try_from(CURRENT_SCHEMA_REVISION)
            .context("Current schema revision cannot be represented as u64")?,
        expected_schema_fingerprint()?,
    )
    .context("Compiled XCZS schema identity violates the xcss contract")
}

async fn insert_root_identity(transaction: &mut Connection, root: RootIdentity) -> Result<()> {
    sqlx::query(
        "INSERT INTO store_meta(key, value) VALUES \
         ('root-device-be', ?1), ('root-inode-be', ?2)",
    )
    .bind(root.device.to_be_bytes().as_slice())
    .bind(root.inode.to_be_bytes().as_slice())
    .execute(&mut *transaction)
    .await
    .map(|result| result.rows_affected())?;
    Ok(())
}

async fn verify_root_identity(connection: &mut Connection, expected: RootIdentity) -> Result<()> {
    let device = load_meta(connection, "root-device-be").await?;
    let inode = load_meta(connection, "root-inode-be").await?;
    ensure!(
        device.as_slice() == expected.device.to_be_bytes(),
        "The state database is bound to a different shared root device"
    );
    ensure!(
        inode.as_slice() == expected.inode.to_be_bytes(),
        "The state database is bound to a different shared root inode"
    );
    Ok(())
}

async fn load_meta(connection: &mut Connection, key: &str) -> Result<Vec<u8>> {
    sqlx::query("SELECT value FROM store_meta WHERE key = ?1")
        .bind(key)
        .try_map(|row: sqlx::sqlite::SqliteRow| row.try_get(0))
        .fetch_optional(&mut *connection)
        .await?
        .ok_or_else(|| anyhow!("State database metadata `{key}` is missing"))
}

async fn recover_database(
    connection: &mut Connection,
    operation_ttl_ms: i64,
    upload_ttl_ms: i64,
    now: i64,
) -> Result<()> {
    let operation_expires_at = expiration_time(now, operation_ttl_ms)?;
    let upload_expires_at = expiration_time(now, upload_ttl_ms)?;
    let mut transaction = connection.begin_with("BEGIN IMMEDIATE").await?;
    super::operation::purge_expired(&mut transaction, now).await?;
    sqlx::query("DELETE FROM operations WHERE state = ?1")
        .bind(OPERATION_RESERVED)
        .execute(&mut *transaction)
        .await
        .map(|result| result.rows_affected())?;
    sqlx::query(
        "UPDATE operations
            SET state = ?1,
                terminal_state = ?2,
                http_status = ?3,
                error_code = ?4,
                updated_at_ms = ?5,
                expires_at_ms = ?6
          WHERE state = ?7",
    )
    .bind(OPERATION_COMPLETED)
    .bind(StoredTerminalState::Unknown as i64)
    .bind(i64::from(UNKNOWN_STATUS))
    .bind(UNKNOWN_CODE)
    .bind(now)
    .bind(operation_expires_at)
    .bind(OPERATION_COMMIT_STARTED)
    .execute(&mut *transaction)
    .await
    .map(|result| result.rows_affected())?;
    sqlx::query(
        "UPDATE operations SET expires_at_ms = ?1
          WHERE state = ?2 AND expires_at_ms > ?1",
    )
    .bind(operation_expires_at)
    .bind(OPERATION_COMPLETED)
    .execute(&mut *transaction)
    .await
    .map(|result| result.rows_affected())?;
    sqlx::query(
        "UPDATE upload_sessions
            SET state = ?1,
                updated_at_ms = ?2,
                expires_at_ms = ?3
          WHERE state = ?4",
    )
    .bind(UPLOAD_UNKNOWN)
    .bind(now)
    .bind(upload_expires_at)
    .bind(UPLOAD_COMMIT_STARTED)
    .execute(&mut *transaction)
    .await
    .map(|result| result.rows_affected())?;
    sqlx::query(
        "UPDATE upload_sessions SET expires_at_ms = ?1
          WHERE expires_at_ms > ?1",
    )
    .bind(upload_expires_at)
    .execute(&mut *transaction)
    .await
    .map(|result| result.rows_affected())?;
    sqlx::query(
        "UPDATE purge_jobs
            SET state = ?1,
                next_attempt_at_ms = ?2,
                updated_at_ms = ?2
          WHERE state = ?3",
    )
    .bind(PURGE_READY)
    .bind(now)
    .bind(PURGE_CLAIMED)
    .execute(&mut *transaction)
    .await
    .map(|result| result.rows_affected())?;
    sqlx::query(
        "UPDATE purge_jobs
            SET next_attempt_at_ms = ?1,
                updated_at_ms = ?1
          WHERE state = ?2 AND next_attempt_at_ms > ?1",
    )
    .bind(now)
    .bind(PURGE_READY)
    .execute(&mut *transaction)
    .await
    .map(|result| result.rows_affected())?;
    transaction.commit().await?;
    Ok(())
}

pub(in crate::server) fn initialize_administrator(
    path: &Path,
    username: &str,
    password: &str,
) -> Result<()> {
    xcss::sqlite::block_on_sqlite_connection(async {
        let mut connection = connect_direct(path).await?;
        let result = async {
            harden_connection(&mut connection).await?;
            configure_validated_connection(&mut connection).await?;
            let mut transaction = connection.begin_with("BEGIN IMMEDIATE").await?;
            crate::auth::initialize_administrator(&mut transaction, username, password).await?;
            transaction.commit().await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        let closed = connection.close().await;
        result?;
        closed?;
        Ok(())
    })
}

pub(in crate::server) fn open_administrator_pool(path: &Path) -> Result<sqlx::SqlitePool> {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .min_connections(0)
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(2))
        .after_connect(|connection, _| {
            Box::pin(async move {
                harden_connection(connection)
                    .await
                    .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
                configure_validated_connection(connection)
                    .await
                    .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
                Ok(())
            })
        })
        .connect_lazy_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(false)
                .busy_timeout(BUSY_TIMEOUT)
                .foreign_keys(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Delete)
                .synchronous(sqlx::sqlite::SqliteSynchronous::Extra),
        );
    Ok(pool)
}
