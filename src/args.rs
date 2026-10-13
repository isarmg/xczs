use anyhow::{Context, Result, bail, ensure};
use clap::{Arg, ArgMatches, Command, value_parser};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fs::Metadata;
use std::net::SocketAddr;
use std::ops::Deref;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use xcss::config::{ConfigSource, EnvMapping, EnvValueKind};

use crate::http_logger::HttpLogger;

const DEFAULT_MAX_UPLOAD_SIZE: u64 = 100 * 1024 * 1024 * 1024;
const DEFAULT_UPLOAD_IDLE_TIMEOUT: u64 = 60;
const DEFAULT_UPLOAD_TOTAL_TIMEOUT: u64 = 24 * 60 * 60;
const DEFAULT_MAX_CONCURRENT_UPLOADS: usize = 4;
const DEFAULT_MIN_FREE_SPACE: u64 = 1024 * 1024 * 1024;
const DEFAULT_MAX_CONNECTIONS: usize = 256;
const DEFAULT_MAX_SEARCH_ENTRIES: usize = 10_000;
const MAX_SEARCH_ENTRIES: usize = 100_000;
const DEFAULT_MAX_CONCURRENT_SEARCHES: usize = 2;
const DEFAULT_REQUEST_TIMEOUT: u64 = 300;
const DEFAULT_TAG_SCAN_INTERVAL_SECONDS: u64 = 3600;
const DEFAULT_MAX_TAG_SCAN_ENTRIES: usize = 1_000_000;
const MAX_TIMEOUT_SECONDS: u64 = 365 * 24 * 60 * 60;
const STATE_DATABASE_FILE_NAME: &str = "state.sqlite3";
const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (git ",
    env!("XCZS_SOURCE_REVISION"),
    ") xcss=",
    env!("XCSS_REVISION")
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ObjectIdentity {
    device: u64,
    inode: u64,
}

impl ObjectIdentity {
    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EntryIdentity {
    parent: ObjectIdentity,
    name: OsString,
}

#[derive(Clone, Debug)]
struct PathIdentity {
    entry_path: PathBuf,
    resolved_path: Option<PathBuf>,
    entry: EntryIdentity,
    object: Option<ObjectIdentity>,
}

impl PathIdentity {
    fn inspect(path: &Path, path_name: &str) -> Result<Self> {
        Self::inspect_with_expected_object(path, path_name, None)
    }

    fn inspect_with_expected_object(
        path: &Path,
        path_name: &str,
        expected_object: Option<ObjectIdentity>,
    ) -> Result<Self> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            env::current_dir()
                .with_context(|| "Failed to determine the current directory")?
                .join(path)
        };
        let file_name = absolute.file_name().ok_or_else(|| {
            anyhow::anyhow!("{path_name} path `{}` must name a file", path.display())
        })?;
        let parent = absolute.parent().ok_or_else(|| {
            anyhow::anyhow!(
                "{path_name} path `{}` must have a parent directory",
                path.display()
            )
        })?;
        let parent = std::fs::canonicalize(parent).with_context(|| {
            format!(
                "Failed to access {path_name} parent directory `{}`",
                parent.display()
            )
        })?;
        let parent_metadata = parent.metadata().with_context(|| {
            format!(
                "Failed to inspect {path_name} parent directory `{}`",
                parent.display()
            )
        })?;
        if !parent_metadata.is_dir() {
            bail!(
                "{path_name} parent `{}` must be a directory",
                parent.display()
            );
        }

        let entry_path = parent.join(file_name);
        let (object, resolved_path) = match std::fs::metadata(&entry_path) {
            Ok(metadata) => {
                let object = ObjectIdentity::from_metadata(&metadata);
                let resolved_path = std::fs::canonicalize(&entry_path).with_context(|| {
                    format!(
                        "Failed to resolve {path_name} path `{}`",
                        entry_path.display()
                    )
                })?;
                let resolved_metadata = resolved_path.metadata().with_context(|| {
                    format!(
                        "Failed to verify resolved {path_name} path `{}`",
                        resolved_path.display()
                    )
                })?;
                if ObjectIdentity::from_metadata(&resolved_metadata) != object {
                    bail!(
                        "{path_name} path `{}` changed while its identity was being inspected",
                        entry_path.display()
                    );
                }
                (Some(object), Some(resolved_path))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (None, None),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Failed to inspect {path_name} path `{}`",
                        entry_path.display()
                    )
                });
            }
        };
        if expected_object.is_some() && expected_object != object {
            bail!(
                "{path_name} path `{}` changed after it was securely read",
                entry_path.display()
            );
        }

        Ok(Self {
            entry_path,
            resolved_path,
            entry: EntryIdentity {
                parent: ObjectIdentity::from_metadata(&parent_metadata),
                name: file_name.to_os_string(),
            },
            object,
        })
    }

    fn shares_entry_or_object_with(&self, other: &Self) -> bool {
        self.entry == other.entry
            || matches!((self.object, other.object), (Some(left), Some(right)) if left == right)
    }

    fn resolves_within_shared_root(
        &self,
        shared_root: &Path,
        shared_root_object: ObjectIdentity,
    ) -> Result<bool> {
        if self.entry_path.starts_with(shared_root)
            || self
                .resolved_path
                .as_deref()
                .is_some_and(|path| path.starts_with(shared_root))
            || self.object == Some(shared_root_object)
        {
            return Ok(true);
        }

        let mut ancestor = self.entry_path.parent();
        while let Some(path) = ancestor {
            let metadata = path.metadata().with_context(|| {
                format!(
                    "Failed to inspect ancestor `{}` while checking the shared-path boundary",
                    path.display()
                )
            })?;
            if ObjectIdentity::from_metadata(&metadata) == shared_root_object {
                return Ok(true);
            }
            ancestor = path.parent();
        }
        Ok(false)
    }
}

fn ensure_outside_shared_root(
    identity: &PathIdentity,
    path_name: &str,
    shared_root: &Path,
) -> Result<()> {
    let shared_root_metadata = shared_root.metadata().with_context(|| {
        format!(
            "Failed to inspect shared path `{}` while validating {path_name}",
            shared_root.display()
        )
    })?;
    if identity.resolves_within_shared_root(
        shared_root,
        ObjectIdentity::from_metadata(&shared_root_metadata),
    )? {
        bail!(
            "{path_name} `{}` must not be inside or resolve into shared path `{}`",
            identity.entry_path.display(),
            shared_root.display()
        );
    }
    Ok(())
}

pub fn build_cli() -> Command {
    Command::new(env!("CARGO_CRATE_NAME"))
        .version(LONG_VERSION)
        .long_version(LONG_VERSION)
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .arg(
            Arg::new("root")
                .long("root")
                .value_parser(value_parser!(PathBuf))
                .default_value("/opt/isarmg/xczs")
                .help("Instance installation root"),
        )
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub struct Args {
    #[serde(skip)]
    pub root: PathBuf,
    pub development: bool,
    #[serde(default = "default_serve_path")]
    pub serve_path: PathBuf,
    #[serde(rename = "data_dir")]
    pub state_dir: Option<PathBuf>,
    pub bind: SocketAddr,
    #[serde(deserialize_with = "deserialize_log_http")]
    #[serde(rename = "log_format")]
    pub http_logger: HttpLogger,
    pub log_level: String,
    pub max_upload_size: u64,
    pub upload_idle_timeout: u64,
    pub upload_total_timeout: u64,
    pub max_concurrent_uploads: usize,
    pub min_free_space: u64,
    pub max_connections: usize,
    pub max_search_entries: usize,
    pub max_concurrent_searches: usize,
    pub request_timeout: u64,
    pub tag_scan_interval_seconds: u64,
    pub max_tag_scan_entries: usize,
    #[serde(skip)]
    pub config_sources: BTreeMap<String, ConfigSource>,
}

/// Startup settings that have passed every cross-field and filesystem check.
///
/// The inner value is private so server internals can hold this type without
/// accidentally constructing an unchecked configuration. Public CLI and
/// library callers can keep using [`Args`]; [`TryFrom`] is available to code
/// that wants an explicit validation boundary.
#[derive(Debug)]
pub struct ValidatedConfig(Args);

impl TryFrom<Args> for ValidatedConfig {
    type Error = anyhow::Error;

    fn try_from(args: Args) -> Result<Self> {
        args.validate().map(Self)
    }
}

impl Deref for ValidatedConfig {
    type Target = Args;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<Args> for ValidatedConfig {
    fn as_ref(&self) -> &Args {
        &self.0
    }
}

impl ValidatedConfig {
    pub(crate) fn state_database_path(&self) -> PathBuf {
        self.0
            .state_dir
            .as_ref()
            .expect("validated configuration requires a persistent state directory")
            .join(STATE_DATABASE_FILE_NAME)
    }
}

impl Default for Args {
    fn default() -> Self {
        Self {
            root: "/opt/isarmg/xczs".into(),
            serve_path: default_serve_path(),
            state_dir: Some(PathBuf::from("/opt/isarmg/xczs/state")),
            bind: default_bind(),
            development: false,
            http_logger: HttpLogger::default(),
            log_level: "info".to_owned(),
            max_upload_size: DEFAULT_MAX_UPLOAD_SIZE,
            upload_idle_timeout: DEFAULT_UPLOAD_IDLE_TIMEOUT,
            upload_total_timeout: DEFAULT_UPLOAD_TOTAL_TIMEOUT,
            max_concurrent_uploads: DEFAULT_MAX_CONCURRENT_UPLOADS,
            min_free_space: DEFAULT_MIN_FREE_SPACE,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            max_search_entries: DEFAULT_MAX_SEARCH_ENTRIES,
            max_concurrent_searches: DEFAULT_MAX_CONCURRENT_SEARCHES,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            tag_scan_interval_seconds: DEFAULT_TAG_SCAN_INTERVAL_SECONDS,
            max_tag_scan_entries: DEFAULT_MAX_TAG_SCAN_ENTRIES,
            config_sources: BTreeMap::new(),
        }
    }
}

impl Args {
    /// Select ROOT and validate its fixed current JSON configuration.
    pub fn parse(matches: ArgMatches) -> Result<Args> {
        let root = matches
            .get_one::<PathBuf>("root")
            .context("root is required")?;
        Self::load(root, &root.join("config/config.json"))?.validate()
    }

    /// Merge the selected instance JSON and environment without creating state.
    pub fn load(root: &Path, config_path: &Path) -> Result<Args> {
        ensure!(
            root.is_absolute()
                && root != Path::new("/")
                && !root.components().any(|part| matches!(
                    part,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )),
            "root must be normalized and absolute"
        );
        let config = read_config(config_path)?;
        let config_identity = PathIdentity::inspect_with_expected_object(
            config_path,
            "Configuration file",
            Some(config.object),
        )?;
        let environment = xcss::config::read_environment(&configuration_environment(), |name| {
            env::var(name).ok()
        })?;
        let defaults = Self {
            state_dir: Some(root.join("state")),
            ..Self::default()
        };
        let loaded = xcss::config::resolve_validated(
            &defaults,
            Some(config.contents.as_bytes()),
            &environment,
            &[],
            validate_intrinsic,
        )?;
        let mut args = loaded.value;
        args.root = root.to_path_buf();
        args.config_sources = loaded.sources;
        args = args.validate_configuration()?;
        ensure_outside_shared_root(&config_identity, "Configuration file", &args.serve_path)?;
        if let Some(state_db) = args.state_database_path()
            && state_db.parent().is_some_and(Path::exists)
        {
            Self::ensure_no_state_db_identity_conflict(
                &state_db,
                &config_identity,
                "Configuration file",
            )?;
        }
        Ok(args)
    }

    /// Validate and normalize settings supplied through JSON or the library API.
    ///
    /// Keeping this boundary on `Args` prevents embedders from bypassing the
    /// invariants that the runtime relies on when it constructs semaphores and
    /// computes request deadlines.
    /// Validate only values, without requiring initialized state or touching databases.
    pub fn validate_configuration(self) -> Result<Self> {
        validate_intrinsic(&self, ConfigSource::File)?;
        ensure!(
            self.serve_path.is_absolute(),
            "serve_path must be an explicitly configured absolute path"
        );
        let state = self.state_dir.as_deref().context("data_dir is required")?;
        ensure!(state.is_absolute(), "data_dir must be an absolute path");
        ensure!(
            !state.starts_with(&self.serve_path) && !self.serve_path.starts_with(state),
            "data_dir and serve_path must not overlap"
        );
        if self.root.is_absolute() {
            ensure!(
                self.serve_path != self.root,
                "shared directory cannot be the installation root"
            );
            for name in [
                "bin",
                "libexec",
                "share",
                "maintenance",
                "config",
                "logs",
                "run",
            ] {
                let protected = self.root.join(name);
                for selected in [&self.serve_path, state] {
                    ensure!(
                        !selected.starts_with(&protected) && !protected.starts_with(selected),
                        "data or shared directory overlaps a private or immutable ROOT namespace"
                    );
                }
            }
        }
        ensure!(
            self.upload_total_timeout >= self.upload_idle_timeout,
            "upload-total-timeout must be greater than or equal to upload-idle-timeout"
        );
        ensure!(
            !self.development || self.bind.ip().is_loopback(),
            "development mode requires a loopback bind address"
        );
        Ok(self)
    }

    pub fn validate(self) -> Result<Self> {
        let mut self_value = self.validate_configuration()?;
        self_value.validate_paths()
    }

    fn validate_paths(&mut self) -> Result<Self> {
        validate_development_web_selection(self)?;
        ensure!(
            self.serve_path.is_absolute(),
            "serve_path must be an explicitly configured absolute path"
        );
        self.serve_path = Self::sanitize_path(&self.serve_path)?;
        self.clone().validate_configuration()?;
        if !self
            .serve_path
            .metadata()
            .with_context(|| {
                format!(
                    "Failed to inspect shared path `{}`",
                    self.serve_path.display()
                )
            })?
            .is_dir()
        {
            bail!(
                "Shared path `{}` must be a directory",
                self.serve_path.display()
            );
        }

        if let Some(state_dir) = self.state_dir.as_deref() {
            self.state_dir = Some(Self::sanitize_state_dir_path(state_dir, &self.serve_path)?);
        }
        let timeout_origin = Instant::now();
        validate_timeout(
            "upload-idle-timeout",
            self.upload_idle_timeout,
            timeout_origin,
        )?;
        validate_timeout(
            "upload-total-timeout",
            self.upload_total_timeout,
            timeout_origin,
        )?;
        if self.upload_total_timeout < self.upload_idle_timeout {
            bail!("upload-total-timeout must be greater than or equal to upload-idle-timeout");
        }
        if self.max_concurrent_uploads == 0 {
            bail!("max-concurrent-uploads must be greater than 0");
        }
        for (name, value) in [
            ("max-connections", self.max_connections),
            ("max-search-entries", self.max_search_entries),
            ("max-concurrent-searches", self.max_concurrent_searches),
        ] {
            if value == 0 {
                bail!("{name} must be greater than 0");
            }
        }
        if self.development && !self.bind.ip().is_loopback() {
            bail!("development mode requires a loopback bind address");
        }
        if self.max_search_entries > MAX_SEARCH_ENTRIES {
            bail!("max-search-entries must not exceed {MAX_SEARCH_ENTRIES}");
        }
        for (name, value) in [
            ("max-connections", self.max_connections),
            ("max-concurrent-uploads", self.max_concurrent_uploads),
            ("max-concurrent-searches", self.max_concurrent_searches),
        ] {
            if value > tokio::sync::Semaphore::MAX_PERMITS {
                bail!(
                    "{name} must not exceed {}",
                    tokio::sync::Semaphore::MAX_PERMITS
                );
            }
        }
        validate_timeout("request-timeout", self.request_timeout, timeout_origin)?;
        if self.tag_scan_interval_seconds < 60
            || self.max_tag_scan_entries == 0
            || self.max_tag_scan_entries > DEFAULT_MAX_TAG_SCAN_ENTRIES
        {
            bail!(
                "tag scan interval must be at least 60 seconds and max-tag-scan-entries must be in 1..=1000000"
            );
        }

        if self.state_dir.is_none() {
            bail!(
                "A persistent SQLite state directory is required; set `data_dir` in ROOT/config/config.json"
            );
        }

        Ok(self.clone())
    }

    fn sanitize_path<P: AsRef<Path>>(path: P) -> Result<PathBuf> {
        let path = path.as_ref();
        if !path.exists() {
            bail!("Path `{}` doesn't exist", path.display());
        }

        std::fs::canonicalize(path)
            .with_context(|| format!("Failed to access path `{}`", path.display()))
    }

    fn sanitize_state_dir_path(path: &Path, serve_path: &Path) -> Result<PathBuf> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            env::current_dir()
                .with_context(|| "Failed to determine the current directory")?
                .join(path)
        };
        // Remove trailing separators and `.` components before lstat. On
        // Linux, a trailing slash otherwise asks the kernel to follow a final
        // symlink to a directory even when using symlink_metadata.
        let absolute = absolute.components().collect::<PathBuf>();
        let metadata = std::fs::symlink_metadata(&absolute)
            .with_context(|| format!("Failed to inspect state directory `{}`", path.display()))?;
        if metadata.file_type().is_symlink() {
            bail!(
                "State directory `{}` must not be a symbolic link",
                path.display()
            );
        }
        if !metadata.file_type().is_dir() {
            bail!("State directory `{}` must be a directory", path.display());
        }

        let state_dir = std::fs::canonicalize(&absolute)
            .with_context(|| format!("Failed to access state directory `{}`", path.display()))?;
        let metadata = state_dir.metadata().with_context(|| {
            format!(
                "Failed to inspect state directory `{}`",
                state_dir.display()
            )
        })?;
        if metadata.uid() != rustix::process::geteuid().as_raw() {
            bail!(
                "State directory `{}` must be owned by the effective service user",
                state_dir.display()
            );
        }
        if metadata.permissions().mode() & 0o7777 != 0o700 {
            bail!(
                "State directory `{}` must have permissions 0700",
                state_dir.display()
            );
        }
        Self::validate_state_dir_ancestor_chain(&state_dir)?;
        if state_dir.starts_with(serve_path) || serve_path.starts_with(&state_dir) {
            bail!(
                "State directory `{}` must not overlap shared path `{}`",
                state_dir.display(),
                serve_path.display()
            );
        }

        // Inspect the fixed target now so a pre-existing symlink, directory,
        // or other unsupported file type fails before the server starts.
        Self::sanitize_state_db_path(&state_dir.join(STATE_DATABASE_FILE_NAME), serve_path)?;
        Ok(state_dir)
    }

    fn validate_state_dir_ancestor_chain(state_dir: &Path) -> Result<()> {
        let service_uid = rustix::process::geteuid().as_raw();
        let trusted_owner = |uid: u32| uid == 0 || uid == service_uid;
        let mut child = state_dir.to_path_buf();
        let mut child_metadata = child.metadata().with_context(|| {
            format!(
                "Failed to inspect state directory `{}` while validating its ancestor chain",
                child.display()
            )
        })?;

        while let Some(parent) = child.parent() {
            let parent_metadata = parent.metadata().with_context(|| {
                format!(
                    "Failed to inspect state directory ancestor `{}`",
                    parent.display()
                )
            })?;
            ensure!(
                parent_metadata.is_dir(),
                "State directory ancestor `{}` must be a directory",
                parent.display()
            );
            ensure!(
                trusted_owner(parent_metadata.uid()),
                "State directory ancestor `{}` must be owned by root or the effective service user",
                parent.display()
            );

            let parent_mode = parent_metadata.permissions().mode() & 0o7777;
            if parent_mode & 0o022 != 0 {
                ensure!(
                    parent_mode & 0o1000 != 0 && trusted_owner(child_metadata.uid()),
                    "State directory ancestor `{}` can be renamed by untrusted local users; group/other-writable ancestors require sticky-bit protection for a root- or service-owned child",
                    parent.display()
                );
            }

            child = parent.to_path_buf();
            child_metadata = parent_metadata;
        }
        Ok(())
    }

    pub(crate) fn state_database_path(&self) -> Option<PathBuf> {
        self.state_dir
            .as_ref()
            .map(|path| path.join(STATE_DATABASE_FILE_NAME))
    }

    fn sanitize_state_db_path(path: &Path, serve_path: &Path) -> Result<PathBuf> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            env::current_dir()
                .with_context(|| "Failed to determine the current directory")?
                .join(path)
        };
        let file_name = absolute.file_name().ok_or_else(|| {
            anyhow::anyhow!("State database path `{}` must name a file", path.display())
        })?;
        let parent = absolute.parent().ok_or_else(|| {
            anyhow::anyhow!(
                "State database path `{}` must have a parent directory",
                path.display()
            )
        })?;
        let parent = std::fs::canonicalize(parent).with_context(|| {
            format!(
                "Failed to access state database parent directory `{}`",
                parent.display()
            )
        })?;
        if !parent
            .metadata()
            .with_context(|| {
                format!(
                    "Failed to inspect state database parent directory `{}`",
                    parent.display()
                )
            })?
            .is_dir()
        {
            bail!(
                "State database parent `{}` must be a directory",
                parent.display()
            );
        }

        let state_db = parent.join(file_name);
        if state_db.starts_with(serve_path) {
            bail!(
                "State database `{}` must not be inside shared path `{}`",
                state_db.display(),
                serve_path.display()
            );
        }

        match std::fs::symlink_metadata(&state_db) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!(
                    "State database `{}` must not be a symbolic link",
                    state_db.display()
                );
            }
            Ok(metadata) if !metadata.file_type().is_file() => {
                bail!(
                    "State database `{}` must be a regular file",
                    state_db.display()
                );
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Failed to inspect state database `{}`", state_db.display())
                });
            }
        }

        Ok(state_db)
    }

    fn ensure_no_state_db_identity_conflict(
        state_db: &Path,
        other: &PathIdentity,
        other_name: &str,
    ) -> Result<()> {
        let conflicts = std::iter::once(state_db.to_path_buf()).chain(
            ["-journal", "-wal", "-shm"].into_iter().map(|suffix| {
                let mut sidecar = state_db.as_os_str().to_os_string();
                sidecar.push(suffix);
                PathBuf::from(sidecar)
            }),
        );
        for conflict in conflicts {
            let identity =
                PathIdentity::inspect(&conflict, "SQLite state database or sidecar file")?;
            if other.shares_entry_or_object_with(&identity) {
                bail!(
                    "{other_name} `{}` conflicts with SQLite state database `{}` or one of its sidecar files by entry or object identity",
                    other.entry_path.display(),
                    state_db.display()
                );
            }
        }
        Ok(())
    }
}

struct ReadConfig {
    contents: String,
    object: ObjectIdentity,
}

impl fmt::Debug for ReadConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReadConfig")
            .field("contents", &"<redacted>")
            .finish()
    }
}

fn read_config(path: &Path) -> Result<ReadConfig> {
    let before = ObjectIdentity::from_metadata(&std::fs::symlink_metadata(path)?);
    let bytes = xcss::config::read_private_file(path)?;
    let after = ObjectIdentity::from_metadata(&std::fs::symlink_metadata(path)?);
    ensure!(
        before == after,
        "Configuration file changed while being read"
    );
    Ok(ReadConfig {
        contents: String::from_utf8(bytes)?,
        object: after,
    })
}

fn validate_timeout(name: &str, seconds: u64, origin: Instant) -> Result<()> {
    if seconds == 0 {
        bail!("{name} must be greater than 0");
    }
    if seconds > MAX_TIMEOUT_SECONDS {
        bail!("{name} must not exceed {MAX_TIMEOUT_SECONDS} seconds");
    }
    if origin.checked_add(Duration::from_secs(seconds)).is_none() {
        bail!("{name} cannot be represented by the platform monotonic clock");
    }
    Ok(())
}

fn deserialize_log_http<'de, D>(deserializer: D) -> Result<HttpLogger, D::Error>
where
    D: Deserializer<'de>,
{
    let value: String = Deserialize::deserialize(deserializer)?;
    value.parse().map_err(serde::de::Error::custom)
}

fn default_serve_path() -> PathBuf {
    PathBuf::new()
}

fn default_bind() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 5000))
}

#[cfg(test)]
mod tests;

fn configuration_environment() -> [EnvMapping<'static>; 17] {
    [
        EnvMapping {
            variable: "XCZS_SERVE_PATH",
            path: "/serve_path",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "XCZS_DATA_DIR",
            path: "/data_dir",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "XCZS_DEVELOPMENT",
            path: "/development",
            kind: EnvValueKind::Boolean,
        },
        EnvMapping {
            variable: "XCZS_BIND",
            path: "/bind",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "XCZS_LOG_FORMAT",
            path: "/log_format",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "XCZS_LOG_LEVEL",
            path: "/log_level",
            kind: EnvValueKind::String,
        },
        EnvMapping {
            variable: "XCZS_MAX_UPLOAD_SIZE",
            path: "/max_upload_size",
            kind: EnvValueKind::UnsignedInteger,
        },
        EnvMapping {
            variable: "XCZS_UPLOAD_IDLE_TIMEOUT",
            path: "/upload_idle_timeout",
            kind: EnvValueKind::UnsignedInteger,
        },
        EnvMapping {
            variable: "XCZS_UPLOAD_TOTAL_TIMEOUT",
            path: "/upload_total_timeout",
            kind: EnvValueKind::UnsignedInteger,
        },
        EnvMapping {
            variable: "XCZS_MAX_CONCURRENT_UPLOADS",
            path: "/max_concurrent_uploads",
            kind: EnvValueKind::UnsignedInteger,
        },
        EnvMapping {
            variable: "XCZS_MIN_FREE_SPACE",
            path: "/min_free_space",
            kind: EnvValueKind::UnsignedInteger,
        },
        EnvMapping {
            variable: "XCZS_MAX_CONNECTIONS",
            path: "/max_connections",
            kind: EnvValueKind::UnsignedInteger,
        },
        EnvMapping {
            variable: "XCZS_MAX_SEARCH_ENTRIES",
            path: "/max_search_entries",
            kind: EnvValueKind::UnsignedInteger,
        },
        EnvMapping {
            variable: "XCZS_MAX_CONCURRENT_SEARCHES",
            path: "/max_concurrent_searches",
            kind: EnvValueKind::UnsignedInteger,
        },
        EnvMapping {
            variable: "XCZS_REQUEST_TIMEOUT",
            path: "/request_timeout",
            kind: EnvValueKind::UnsignedInteger,
        },
        EnvMapping {
            variable: "XCZS_TAG_SCAN_INTERVAL_SECONDS",
            path: "/tag_scan_interval_seconds",
            kind: EnvValueKind::UnsignedInteger,
        },
        EnvMapping {
            variable: "XCZS_MAX_TAG_SCAN_ENTRIES",
            path: "/max_tag_scan_entries",
            kind: EnvValueKind::UnsignedInteger,
        },
    ]
}

fn validate_development_web_selection(args: &Args) -> Result<()> {
    let Some(directory) = env::var_os("XCZS_DEV_WEB_DIR") else {
        return Ok(());
    };
    let invalid = || {
        xcss::config::ConfigError::new(
            xcss::config::Reason::InvalidValue,
            "/XCZS_DEV_WEB_DIR",
            ConfigSource::Environment,
        )
    };
    if !args.development || env!("XCZS_SOURCE_REVISION") != "unbound" {
        return Err(invalid().into());
    }
    // Validate the entire selected development tree before initialization or
    // runtime locks/logs. Runtime construction captures and checks it again.
    xcss::web_assets::DirectoryAssets::new(Path::new(&directory)).map_err(|_| invalid())?;
    Ok(())
}

fn validate_intrinsic(args: &Args, source: ConfigSource) -> Result<(), xcss::config::ConfigError> {
    let invalid =
        |path| xcss::config::ConfigError::new(xcss::config::Reason::InvalidValue, path, source);
    if !matches!(
        args.log_level.as_str(),
        "off" | "error" | "warn" | "info" | "debug" | "trace"
    ) {
        return Err(invalid("/log_level"));
    }
    if args.state_dir.is_none() {
        return Err(invalid("/data_dir"));
    }
    if !args.serve_path.as_os_str().is_empty() && !args.serve_path.is_absolute() {
        return Err(invalid("/serve_path"));
    }
    for (path, value) in [
        ("/max_concurrent_uploads", args.max_concurrent_uploads),
        ("/max_connections", args.max_connections),
        ("/max_search_entries", args.max_search_entries),
        ("/max_concurrent_searches", args.max_concurrent_searches),
    ] {
        if value == 0 || value > tokio::sync::Semaphore::MAX_PERMITS {
            return Err(invalid(path));
        }
    }
    if args.max_search_entries > MAX_SEARCH_ENTRIES {
        return Err(invalid("/max_search_entries"));
    }
    let now = Instant::now();
    for (path, value) in [
        ("/upload_idle_timeout", args.upload_idle_timeout),
        ("/upload_total_timeout", args.upload_total_timeout),
        ("/request_timeout", args.request_timeout),
    ] {
        validate_timeout(path, value, now).map_err(|_| invalid(path))?;
    }
    if args.tag_scan_interval_seconds < 60 {
        return Err(invalid("/tag_scan_interval_seconds"));
    }
    if args.max_tag_scan_entries == 0 || args.max_tag_scan_entries > DEFAULT_MAX_TAG_SCAN_ENTRIES {
        return Err(invalid("/max_tag_scan_entries"));
    }
    Ok(())
}
