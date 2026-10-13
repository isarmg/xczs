use super::*;

use assert_fs::prelude::*;
use std::ffi::OsStr;
use std::os::unix::fs::OpenOptionsExt;

fn private_state_dir() -> assert_fs::TempDir {
    let state_dir = assert_fs::TempDir::new().unwrap();
    std::fs::set_permissions(state_dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    state_dir
}

fn make_config_private(path: &Path) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::set_permissions(
        path.parent().unwrap(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
}

fn matches_with_state<I, T>(values: I, state_dir: &Path) -> ArgMatches
where
    I: IntoIterator<Item = T>,
    T: AsRef<OsStr>,
{
    let mut values = values
        .into_iter()
        .map(|value| value.as_ref().to_os_string())
        .collect::<Vec<_>>();
    if values
        .get(1)
        .is_some_and(|value| !value.to_string_lossy().starts_with('-'))
    {
        values.insert(1, "--serve-path".into());
    }
    let has_config = values.iter().any(|value| {
        value.to_str().is_some_and(|value| {
            value == "--config" || value == "-c" || value.starts_with("--config=")
        })
    });
    if !has_config {
        let config_path = state_dir.join("test-auth.json");
        if !config_path.exists() {
            let mut config = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&config_path)
                .unwrap();
            serde_json::to_writer(&mut config, &serde_json::json!({})).unwrap();
            std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        values.push("--config".into());
        values.push(config_path.into_os_string());
    }
    let config_has_root = values
        .iter()
        .position(|value| value == "--config" || value == "-c")
        .and_then(|index| values.get(index + 1))
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|value| value.get("serve_path").is_some());
    if !values.iter().any(|value| value == "--serve-path") && !config_has_root {
        values.push("--serve-path".into());
        values.push(std::env::current_dir().unwrap().into_os_string());
    }
    values.push("--data-dir".into());
    values.push(state_dir.as_os_str().to_os_string());
    if !values.iter().any(|value| {
        value == "run"
            || value == "init"
            || value == "config"
            || value == "check"
            || value == "status"
            || value == "admin"
    }) {
        values.insert(1, "run".into());
    }
    build_cli().try_get_matches_from(values).unwrap()
}

#[test]
fn test_default() {
    let state_dir = private_state_dir();
    let matches = matches_with_state([""], state_dir.path());
    let args = Args::parse(matches).unwrap();
    let cwd = Args::sanitize_path(std::env::current_dir().unwrap()).unwrap();
    assert_eq!(args.serve_path, cwd);
    assert_eq!(args.bind, default_bind());
    assert!(!args.development);
    assert_eq!(args.max_upload_size, DEFAULT_MAX_UPLOAD_SIZE);
    assert_eq!(args.upload_idle_timeout, DEFAULT_UPLOAD_IDLE_TIMEOUT);
    assert_eq!(args.upload_total_timeout, DEFAULT_UPLOAD_TOTAL_TIMEOUT);
    assert_eq!(args.max_concurrent_uploads, DEFAULT_MAX_CONCURRENT_UPLOADS);
    assert_eq!(args.min_free_space, DEFAULT_MIN_FREE_SPACE);
    assert_eq!(args.max_connections, DEFAULT_MAX_CONNECTIONS);
    assert_eq!(args.max_search_entries, DEFAULT_MAX_SEARCH_ENTRIES);
    assert_eq!(
        args.max_concurrent_searches,
        DEFAULT_MAX_CONCURRENT_SEARCHES
    );
    assert_eq!(args.request_timeout, DEFAULT_REQUEST_TIMEOUT);
}

#[test]
fn empty_config_file_is_invalid_json() {
    let tmpdir = assert_fs::TempDir::new().unwrap();
    let config_file = tmpdir.child("config.json");
    config_file.write_str("").unwrap();
    make_config_private(config_file.path());

    let cli = build_cli();
    let matches = cli
        .try_get_matches_from(vec!["", "run", "-c", &config_file.to_string_lossy()])
        .unwrap();
    let err = Args::parse(matches).unwrap_err();
    assert!(
        err.downcast_ref::<xcss::config::ConfigError>()
            .is_some_and(|error| error.reason == xcss::config::Reason::InvalidSyntax)
    );
}

#[test]
fn development_mode_is_explicit_and_requires_only_loopback_bindings() {
    for bind in ["127.0.0.1:5000", "[::1]:5000"] {
        let state_dir = private_state_dir();
        let matches = matches_with_state(["", "--development", "--bind", bind], state_dir.path());
        let args = Args::parse(matches).unwrap();
        assert!(args.development);
        assert!(args.bind.ip().is_loopback());
    }
    for bind in ["0.0.0.0:5000", "[::]:5000", "192.0.2.1:5000"] {
        let state_dir = private_state_dir();
        let matches = matches_with_state(["", "--development", "--bind", bind], state_dir.path());
        assert!(
            Args::parse(matches)
                .unwrap_err()
                .to_string()
                .contains("loopback")
        );
    }
    assert!(
        build_cli()
            .try_get_matches_from(["", "--trusted-proxy", "127.0.0.1"])
            .is_err()
    );
}

#[test]
fn oversized_config_file_is_rejected_before_parsing() {
    let tmpdir = assert_fs::TempDir::new().unwrap();
    let config_file = tmpdir.child("config.json");
    config_file
        .write_binary(&vec![b' '; xcss::config::MAX_CONFIG_BYTES + 1])
        .unwrap();
    make_config_private(config_file.path());

    let matches = build_cli()
        .try_get_matches_from(vec!["", "run", "-c", &config_file.to_string_lossy()])
        .unwrap();
    let error = Args::parse(matches).expect_err("an oversized config was accepted");
    assert!(
        error
            .downcast_ref::<xcss::config::ConfigError>()
            .is_some_and(|error| error.reason == xcss::config::Reason::LimitExceeded),
        "unexpected error: {error:#}"
    );
}

#[test]
fn symbolic_link_config_file_is_rejected() {
    let tmpdir = assert_fs::TempDir::new().unwrap();
    let target = tmpdir.child("target.json");
    let config_file = tmpdir.child("config.json");
    target
        .write_str(&serde_json::json!({}).to_string())
        .unwrap();
    std::os::unix::fs::symlink(target.path(), config_file.path()).unwrap();

    let error = read_config(config_file.path()).expect_err("a symlink config was accepted");
    assert!(
        error.downcast_ref::<xcss::config::ConfigError>().is_some(),
        "unexpected error: {error:#}"
    );
}

#[test]
fn read_config_rejects_a_real_hard_link_and_insecure_mode() {
    let tmpdir = assert_fs::TempDir::new().unwrap();
    let insecure = tmpdir.child("insecure.json");
    insecure.write_str("auth: []\n").unwrap();
    std::fs::set_permissions(insecure.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
    let mode_error = read_config(insecure.path()).expect_err("a 0644 config was accepted");
    assert!(
        mode_error
            .downcast_ref::<xcss::config::ConfigError>()
            .is_some()
    );

    let linked = tmpdir.child("linked.json");
    linked.write_str("auth: []\n").unwrap();
    make_config_private(linked.path());
    std::fs::hard_link(linked.path(), tmpdir.child("alias.json").path()).unwrap();
    let link_error = read_config(linked.path()).expect_err("a hard-linked config was accepted");
    assert!(
        link_error
            .downcast_ref::<xcss::config::ConfigError>()
            .is_some()
    );
}

#[test]
fn path_identity_matches_canonical_entries_and_existing_objects() {
    let tmpdir = assert_fs::TempDir::new().unwrap();
    let real_parent = tmpdir.child("real");
    std::fs::create_dir(real_parent.path()).unwrap();
    let alias_parent = tmpdir.child("alias");
    std::os::unix::fs::symlink(real_parent.path(), alias_parent.path()).unwrap();

    let direct_entry =
        PathIdentity::inspect(&real_parent.path().join("future"), "test path").unwrap();
    let aliased_entry =
        PathIdentity::inspect(&alias_parent.path().join("future"), "test path").unwrap();
    assert!(direct_entry.shares_entry_or_object_with(&aliased_entry));
    assert!(direct_entry.object.is_none());

    let object = real_parent.child("object");
    object.write_str("contents").unwrap();
    let hard_link = tmpdir.child("object-link");
    std::fs::hard_link(object.path(), hard_link.path()).unwrap();
    let direct_object = PathIdentity::inspect(object.path(), "test path").unwrap();
    let aliased_object = PathIdentity::inspect(hard_link.path(), "test path").unwrap();
    assert_ne!(direct_object.entry, aliased_object.entry);
    assert!(direct_object.shares_entry_or_object_with(&aliased_object));
}

#[test]
fn configuration_cannot_resolve_into_the_shared_root() {
    let shared = assert_fs::TempDir::new().unwrap();
    let state_dir = private_state_dir();
    let alias_dir = assert_fs::TempDir::new().unwrap();
    let shared_alias = alias_dir.child("shared-alias");
    std::os::unix::fs::symlink(shared.path(), shared_alias.path()).unwrap();

    let config = shared.child("config.json");
    config
        .write_str(&serde_json::json!({"data_dir":state_dir.path()}).to_string())
        .unwrap();
    make_config_private(config.path());
    for config_path in [
        config.path().to_path_buf(),
        shared_alias.path().join("config.json"),
    ] {
        let matches = build_cli()
            .try_get_matches_from([
                OsString::from("xczs"),
                OsString::from("run"),
                OsString::from("--serve-path"),
                shared.path().as_os_str().to_owned(),
                OsString::from("--config"),
                config_path.into_os_string(),
            ])
            .unwrap();
        let error = Args::parse(matches)
            .expect_err("a configuration path inside the shared root was accepted");
        assert!(
            error.to_string().contains("resolve into shared path")
                || error
                    .downcast_ref::<xcss::config::ConfigError>()
                    .is_some_and(|error| error.reason == xcss::config::Reason::UnsafeFile),
            "unexpected error: {error:#}"
        );
    }
}

#[test]
fn state_directory_rejects_a_renameable_ancestor_chain() {
    let shared = assert_fs::TempDir::new().unwrap();
    let container = assert_fs::TempDir::new().unwrap();
    let writable_parent = container.child("writable-parent");
    std::fs::create_dir(writable_parent.path()).unwrap();
    std::fs::set_permissions(
        writable_parent.path(),
        std::fs::Permissions::from_mode(0o777),
    )
    .unwrap();
    let state_dir = writable_parent.child("state");
    std::fs::create_dir(state_dir.path()).unwrap();
    std::fs::set_permissions(state_dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();

    let validate = || {
        Args {
            serve_path: shared.path().to_path_buf(),
            state_dir: Some(state_dir.path().to_path_buf()),
            ..Args::default()
        }
        .validate()
    };
    let error = validate().expect_err("a state directory under a renameable parent was accepted");
    assert!(
        error
            .to_string()
            .contains("can be renamed by untrusted local users"),
        "unexpected error: {error:#}"
    );

    std::fs::set_permissions(
        writable_parent.path(),
        std::fs::Permissions::from_mode(0o1777),
    )
    .unwrap();
    validate().expect("a sticky parent protecting a service-owned child was rejected");
}

#[test]
fn direct_validation_enforces_runtime_invariants() {
    let state_dir = private_state_dir();
    let tmpdir = assert_fs::TempDir::new().unwrap();
    let args = Args {
        serve_path: tmpdir.path().to_path_buf(),
        max_concurrent_uploads: 0,
        state_dir: Some(state_dir.path().to_path_buf()),
        ..Args::default()
    };
    let error = args
        .validate()
        .expect_err("library configuration bypassed validation");
    assert_eq!(
        error
            .downcast_ref::<xcss::config::ConfigError>()
            .unwrap()
            .path,
        "/max_concurrent_uploads"
    );
}

#[test]
fn validated_config_exposes_only_a_normalized_read_view() {
    let tmpdir = assert_fs::TempDir::new().unwrap();
    let state_dir = private_state_dir();
    let relative = tmpdir.path().join(".");
    let args = Args {
        serve_path: relative,
        state_dir: Some(state_dir.path().to_path_buf()),
        ..Args::default()
    };
    let config = ValidatedConfig::try_from(args).unwrap();
    assert!(config.serve_path.is_absolute());
    assert!(config.serve_path.is_dir());
}

#[test]
fn config_without_bind_uses_the_loopback_default() {
    let tmpdir = assert_fs::TempDir::new().unwrap();
    let state_dir = private_state_dir();
    let config_file = tmpdir.child("config.json");
    config_file
        .write_str(&serde_json::json!({}).to_string())
        .unwrap();
    make_config_private(config_file.path());

    let matches = matches_with_state(["", "-c", &config_file.to_string_lossy()], state_dir.path());
    let args = Args::parse(matches).unwrap();
    assert_eq!(args.bind, default_bind());
}

#[test]
fn test_args_from_config_file1() {
    let tmpdir = assert_fs::TempDir::new().unwrap();
    let config_dir = assert_fs::TempDir::new().unwrap();
    let state_dir = private_state_dir();
    let config_file = config_dir.child("config.json");
    let contents =
        serde_json::json!({"serve_path":tmpdir.path(),"bind":"0.0.0.0:3000"}).to_string();
    config_file.write_str(&contents).unwrap();
    make_config_private(config_file.path());

    let matches = matches_with_state(["", "-c", &config_file.to_string_lossy()], state_dir.path());
    let args = Args::parse(matches).unwrap();
    assert_eq!(args.serve_path, Args::sanitize_path(&tmpdir).unwrap());
    assert_eq!(args.bind, "0.0.0.0:3000".parse::<SocketAddr>().unwrap());
}

#[test]
fn test_file_serve_path_from_config_is_rejected() {
    let tmpdir = assert_fs::TempDir::new().unwrap();
    let shared_file = tmpdir.child("shared.txt");
    shared_file.write_str("contents").unwrap();
    let config_file = tmpdir.child("config.json");
    let contents = serde_json::json!({"serve_path":shared_file.path()}).to_string();
    config_file.write_str(&contents).unwrap();
    make_config_private(config_file.path());

    let cli = build_cli();
    let matches = cli
        .try_get_matches_from(vec!["", "run", "-c", &config_file.to_string_lossy()])
        .unwrap();
    let err = Args::parse(matches).unwrap_err();
    assert!(
        err.to_string().contains("must be a directory"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn multiple_bind_addresses_are_rejected() {
    let tmpdir = assert_fs::TempDir::new().unwrap();
    let state_dir = private_state_dir();
    let config_file = tmpdir.child("config.json");
    let contents = serde_json::json!({"bind":["127.0.0.1","192.168.8.10"]}).to_string();
    config_file.write_str(&contents).unwrap();
    make_config_private(config_file.path());

    let matches = matches_with_state(["", "-c", &config_file.to_string_lossy()], state_dir.path());
    assert!(Args::parse(matches).is_err());
}

#[test]
fn empty_bind_list_is_rejected() {
    let tmpdir = assert_fs::TempDir::new().unwrap();
    let config_file = tmpdir.child("config.json");
    config_file
        .write_str(&serde_json::json!({"bind":[]}).to_string())
        .unwrap();
    make_config_private(config_file.path());

    let matches = build_cli()
        .try_get_matches_from(vec!["", "run", "-c", &config_file.to_string_lossy()])
        .unwrap();
    let error = Args::parse(matches).expect_err("an empty bind list was accepted");
    assert!(
        error
            .downcast_ref::<xcss::config::ConfigError>()
            .is_some_and(|error| error.path == "/bind")
    );
}

#[test]
fn test_non_ip_bind_config_is_rejected() {
    let tmpdir = assert_fs::TempDir::new().unwrap();
    let config_file = tmpdir.child("config.json");
    let contents =
        serde_json::json!({"serve_path":tmpdir.path(),"bind":"not-an-ip-address:5000"}).to_string();
    config_file.write_str(&contents).unwrap();
    make_config_private(config_file.path());

    let cli = build_cli();
    let matches = cli
        .try_get_matches_from(vec!["", "run", "-c", &config_file.to_string_lossy()])
        .unwrap();
    let err = Args::parse(matches).unwrap_err();
    assert!(
        err.downcast_ref::<xcss::config::ConfigError>()
            .is_some_and(|error| error.path == "/bind")
    );
}

#[test]
fn upload_limits_from_cli_override_defaults() {
    let state_dir = private_state_dir();
    let matches = matches_with_state(
        [
            "",
            "--max-upload-size",
            "0",
            "--upload-idle-timeout",
            "12",
            "--upload-total-timeout",
            "34",
            "--max-concurrent-uploads",
            "2",
            "--min-free-space",
            "0",
            "--max-connections",
            "11",
            "--max-search-entries",
            "12",
            "--max-concurrent-searches",
            "3",
            "--request-timeout",
            "16",
        ],
        state_dir.path(),
    );
    let args = Args::parse(matches).unwrap();
    assert_eq!(args.max_upload_size, 0);
    assert_eq!(args.upload_idle_timeout, 12);
    assert_eq!(args.upload_total_timeout, 34);
    assert_eq!(args.max_concurrent_uploads, 2);
    assert_eq!(args.min_free_space, 0);
    assert_eq!(args.max_connections, 11);
    assert_eq!(args.max_search_entries, 12);
    assert_eq!(args.max_concurrent_searches, 3);
    assert_eq!(args.request_timeout, 16);
}

#[test]
fn invalid_upload_time_and_concurrency_limits_are_rejected() {
    for invalid_args in [
        ["--upload-idle-timeout", "0"],
        ["--upload-total-timeout", "0"],
        ["--max-concurrent-uploads", "0"],
        ["--upload-idle-timeout", "60"],
    ] {
        let state_dir = private_state_dir();
        let mut values = vec![""];
        values.extend(invalid_args);
        if invalid_args[0] == "--upload-idle-timeout" && invalid_args[1] == "60" {
            values.extend(["--upload-total-timeout", "59"]);
        }
        let matches = matches_with_state(values, state_dir.path());
        assert!(Args::parse(matches).is_err(), "accepted {invalid_args:?}");
    }
}

#[test]
fn extreme_timeout_values_are_rejected_during_startup_validation() {
    for name in [
        "--upload-idle-timeout",
        "--upload-total-timeout",
        "--request-timeout",
    ] {
        for value in [(MAX_TIMEOUT_SECONDS + 1).to_string(), u64::MAX.to_string()] {
            let state_dir = private_state_dir();
            let mut values = vec!["".to_string(), name.to_string(), value];
            if name == "--upload-idle-timeout" {
                values.extend([
                    "--upload-total-timeout".to_string(),
                    MAX_TIMEOUT_SECONDS.to_string(),
                ]);
            }
            let matches = matches_with_state(values, state_dir.path());
            let error = Args::parse(matches).expect_err("extreme timeout value was accepted");
            assert!(
                error
                    .downcast_ref::<xcss::config::ConfigError>()
                    .is_some_and(|error| error.path
                        == format!("/{}", name.trim_start_matches("--").replace('-', "_"))),
                "unexpected error for {name}: {error:#}"
            );
        }
    }
}

#[test]
fn maximum_timeout_value_is_accepted() {
    let state_dir = private_state_dir();
    let maximum = MAX_TIMEOUT_SECONDS.to_string();
    let matches = matches_with_state(
        [
            "",
            "--upload-idle-timeout",
            maximum.as_str(),
            "--upload-total-timeout",
            maximum.as_str(),
            "--request-timeout",
            maximum.as_str(),
        ],
        state_dir.path(),
    );
    let args = Args::parse(matches).unwrap();
    assert_eq!(args.upload_idle_timeout, MAX_TIMEOUT_SECONDS);
    assert_eq!(args.upload_total_timeout, MAX_TIMEOUT_SECONDS);
    assert_eq!(args.request_timeout, MAX_TIMEOUT_SECONDS);
}

#[test]
fn zero_ordinary_request_budgets_are_rejected() {
    for name in [
        "--max-connections",
        "--max-search-entries",
        "--max-concurrent-searches",
        "--request-timeout",
    ] {
        let state_dir = private_state_dir();
        let matches = matches_with_state(["", name, "0"], state_dir.path());
        assert!(Args::parse(matches).is_err(), "accepted {name}=0");
    }
}

#[test]
fn search_entry_limit_has_a_hard_upper_bound() {
    let state_dir = private_state_dir();
    for value in [MAX_SEARCH_ENTRIES, MAX_SEARCH_ENTRIES + 1] {
        let value_string = value.to_string();
        let matches = matches_with_state(
            ["", "--max-search-entries", value_string.as_str()],
            state_dir.path(),
        );
        let result = Args::parse(matches);
        if value == MAX_SEARCH_ENTRIES {
            assert!(result.is_ok());
        } else {
            assert!(
                result
                    .expect_err("an excessive search entry limit was accepted")
                    .downcast_ref::<xcss::config::ConfigError>()
                    .is_some_and(|error| error.path == "/max_search_entries")
            );
        }
    }
}

#[test]
fn semaphore_limits_above_tokios_maximum_are_rejected() {
    let too_many = (tokio::sync::Semaphore::MAX_PERMITS + 1).to_string();
    for name in [
        "--max-connections",
        "--max-concurrent-uploads",
        "--max-concurrent-searches",
    ] {
        let state_dir = private_state_dir();
        let matches = matches_with_state(["", name, too_many.as_str()], state_dir.path());
        let error = Args::parse(matches).expect_err("oversized semaphore count was accepted");
        assert!(
            error
                .downcast_ref::<xcss::config::ConfigError>()
                .is_some_and(|error| error.path
                    == format!("/{}", name.trim_start_matches("--").replace('-', "_"))),
            "unexpected error for {name}: {error:#}"
        );
    }
}

#[test]
fn shared_root_must_be_explicit_and_absolute() {
    for serve_path in [
        PathBuf::new(),
        PathBuf::from("."),
        PathBuf::from("relative"),
    ] {
        let error = Args {
            serve_path,
            ..Args::default()
        }
        .validate()
        .unwrap_err();
        assert!(
            error.to_string().contains("absolute")
                || error
                    .downcast_ref::<xcss::config::ConfigError>()
                    .is_some_and(|error| error.path == "/serve_path"),
            "{error:#}"
        );
    }
}

#[test]
fn removed_port_and_array_bind_are_not_aliases() {
    for args in [
        vec!["xczs", "--port", "5000"],
        vec!["xczs", "--bind", "127.0.0.1"],
        vec!["xczs", "--bind", "127.0.0.1:1,127.0.0.2:1"],
    ] {
        assert!(build_cli().try_get_matches_from(args).is_err());
    }
    assert!(serde_json::from_str::<Args>(r#"{"port":5000}"#).is_err());
    assert!(serde_json::from_str::<Args>(r#"{"bind":["127.0.0.1"]}"#).is_err());
}

#[test]
fn private_reader_requires_owned_0600_file_and_0700_parent() {
    let directory = private_state_dir();
    let file = directory.child("config.json");
    file.write_str("{}").unwrap();
    make_config_private(file.path());
    assert!(read_config(file.path()).is_ok());
    for mode in [0o400, 0o440, 0o640, 0o644] {
        std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(mode)).unwrap();
        assert!(read_config(file.path()).is_err());
    }
    make_config_private(file.path());
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o750)).unwrap();
    assert!(read_config(file.path()).is_err());
}

#[test]
fn common_defaults_and_config_selection_are_explicit() {
    let args = Args::default();
    assert_eq!(
        args.state_dir.as_deref(),
        Some(Path::new("/var/lib/isarmg/xczs"))
    );
    assert!(!args.development);
    assert_eq!(args.bind, "127.0.0.1:5000".parse().unwrap());
    assert!(args.serve_path.as_os_str().is_empty());
    let matches = build_cli().try_get_matches_from(["xczs", "run"]).unwrap();
    assert_eq!(
        matches.get_one::<PathBuf>("config").unwrap(),
        Path::new("/etc/isarmg/xczs/config.json")
    );
    let temp = private_state_dir();
    let missing = temp.path().join("missing.json");
    let matches = build_cli()
        .try_get_matches_from([
            OsString::from("xczs"),
            OsString::from("run"),
            OsString::from("--config"),
            missing.clone().into_os_string(),
        ])
        .unwrap();
    assert_eq!(selected_config_path(&matches).unwrap(), Some(missing));
    assert!(Args::parse(matches).is_err());
}

#[test]
fn development_false_can_override_true_from_json() {
    let shared = private_state_dir();
    let config_dir = private_state_dir();
    let state = private_state_dir();
    let config = config_dir.child("config.json");
    config
        .write_str(
            &serde_json::json!({"serve_path": shared.path(), "development": true}).to_string(),
        )
        .unwrap();
    make_config_private(config.path());
    let matches = matches_with_state(
        [
            "xczs",
            "--config",
            &config.path().to_string_lossy(),
            "--development=false",
        ],
        state.path(),
    );
    assert!(!Args::parse(matches).unwrap().development);
}

#[test]
fn invalid_low_priority_paths_cannot_be_hidden_by_cli_overrides() {
    for config in [r#"{"data_dir":null}"#, r#"{"serve_path":"relative"}"#] {
        let result = xcss::config::resolve_validated(
            &Args::default(),
            Some(config.as_bytes()),
            &[],
            &[
                Override::new("/serve_path", "/srv/xczs"),
                Override::new("/data_dir", "/var/lib/isarmg/xczs"),
            ],
            validate_intrinsic,
        );
        assert!(result.is_err());
    }
}
