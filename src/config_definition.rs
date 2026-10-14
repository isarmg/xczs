//! Current JSON configuration data used by runtime and package contract generation.
use serde::{Deserialize, Deserializer, Serialize};
use std::{collections::BTreeMap, net::SocketAddr, path::PathBuf};

pub const DEFAULT_LOG_FORMAT: &str = r#"$time_iso8601 $log_level - "$request" $status operation_id=$operation_id operation_state=$operation_state request_id=$request_id"#;
pub(crate) const DEFAULT_MAX_UPLOAD_SIZE: u64 = 100 * 1024 * 1024 * 1024;
pub(crate) const DEFAULT_UPLOAD_IDLE_TIMEOUT: u64 = 60;
pub(crate) const DEFAULT_UPLOAD_TOTAL_TIMEOUT: u64 = 24 * 60 * 60;
pub(crate) const DEFAULT_MAX_CONCURRENT_UPLOADS: usize = 4;
pub(crate) const DEFAULT_MIN_FREE_SPACE: u64 = 1024 * 1024 * 1024;
pub(crate) const DEFAULT_MAX_CONNECTIONS: usize = 256;
pub(crate) const DEFAULT_MAX_SEARCH_ENTRIES: usize = 10_000;
pub(crate) const MAX_SEARCH_ENTRIES: usize = 100_000;
pub(crate) const DEFAULT_MAX_CONCURRENT_SEARCHES: usize = 2;
pub(crate) const DEFAULT_REQUEST_TIMEOUT: u64 = 300;
pub(crate) const DEFAULT_TAG_SCAN_INTERVAL_SECONDS: u64 = 3600;
pub(crate) const DEFAULT_MAX_TAG_SCAN_ENTRIES: usize = 1_000_000;
pub(crate) const MAX_TIMEOUT_SECONDS: u64 = 365 * 24 * 60 * 60;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
#[serde(default)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
#[serde(bound(deserialize = "L: std::str::FromStr, L::Err: std::fmt::Display + std::fmt::Debug"))]
#[schemars(
    bound = "L: serde::Serialize + std::str::FromStr, L::Err: std::fmt::Display + std::fmt::Debug"
)]
pub struct ArgsDefinition<L, S> {
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
    #[schemars(with = "String")]
    pub http_logger: L,
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
    pub config_sources: BTreeMap<String, S>,
}

impl<L, S> Default for ArgsDefinition<L, S>
where
    L: std::str::FromStr,
    L::Err: std::fmt::Debug,
{
    fn default() -> Self {
        Self {
            root: "/opt/isarmg/xczs".into(),
            serve_path: default_serve_path(),
            state_dir: Some(PathBuf::from("/opt/isarmg/xczs/state")),
            bind: default_bind(),
            development: false,
            http_logger: DEFAULT_LOG_FORMAT
                .parse()
                .expect("current default log format"),
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

fn deserialize_log_http<'de, D, L>(deserializer: D) -> Result<L, D::Error>
where
    D: Deserializer<'de>,
    L: std::str::FromStr,
    L::Err: std::fmt::Display,
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

pub(crate) fn data_path_spec() -> serde_json::Value {
    serde_json::json!({"base":"config","value":"/data_dir","fallback":null,"suffix":"","encoding":"path"})
}
pub(crate) fn shared_path_spec() -> serde_json::Value {
    serde_json::json!({"base":"config","value":"/serve_path","fallback":null,"suffix":"","encoding":"path"})
}
pub(crate) fn current_rules(
    final_configuration: bool,
    include_formatter: bool,
) -> serde_json::Value {
    use serde_json::json;
    let mut rules = vec![
        json!({"kind":"enum","fields":["/log_level"],"arguments":{"values":["off","error","warn","info","debug","trace"]}}),
        json!({"kind":"socket-address","fields":["/bind"],"arguments":{"loopback_when":"/development"}}),
        json!({"kind":"path","fields":["/data_dir"],"arguments":{"absolute":true}}),
        json!({"kind":"path","fields":["/serve_path"],"arguments":{"absolute":true,"optional":!final_configuration}}),
    ];
    for (field, maximum) in [
        (
            "/max_concurrent_uploads",
            tokio::sync::Semaphore::MAX_PERMITS,
        ),
        ("/max_connections", tokio::sync::Semaphore::MAX_PERMITS),
        ("/max_search_entries", MAX_SEARCH_ENTRIES),
        (
            "/max_concurrent_searches",
            tokio::sync::Semaphore::MAX_PERMITS,
        ),
        ("/max_tag_scan_entries", DEFAULT_MAX_TAG_SCAN_ENTRIES),
    ] {
        rules.push(json!({"kind":"integer-range","fields":[field],"arguments":{"minimum":1,"maximum":maximum}}));
    }
    for field in [
        "/upload_idle_timeout",
        "/upload_total_timeout",
        "/request_timeout",
    ] {
        rules.push(json!({"kind":"integer-range","fields":[field],"arguments":{"minimum":1,"maximum":MAX_TIMEOUT_SECONDS}}));
    }
    rules.push(json!({"kind":"integer-range","fields":["/tag_scan_interval_seconds"],"arguments":{"minimum":60,"maximum":MAX_TIMEOUT_SECONDS}}));
    if final_configuration {
        rules.push(json!({"kind":"root-state-paths","fields":[],"arguments":{"paths":[data_path_spec(),shared_path_spec()],"protected":xcss::server_layout::ROOT_RESERVED_DIRECTORIES}}));
        rules.push(json!({"kind":"integer-order","fields":["/upload_idle_timeout","/upload_total_timeout"],"arguments":{"relation":"less-or-equal"}}));
    }
    if include_formatter {
        rules.push(crate::log_format_definition::rule());
    }
    json!(rules)
}
