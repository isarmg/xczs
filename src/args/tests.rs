use super::*;

use assert_fs::prelude::*;

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

use xcss::config::Override;

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
fn validated_config_requires_a_canonical_absolute_path() {
    let tmpdir = assert_fs::TempDir::new().unwrap();
    let state_dir = private_state_dir();
    let relative = tmpdir.path().join(".");
    let args = Args {
        serve_path: relative,
        state_dir: Some(state_dir.path().to_path_buf()),
        ..Args::default()
    };
    let error = ValidatedConfig::try_from(args.clone()).unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<xcss::config::ConfigError>()
            .unwrap()
            .path,
        "/serve_path"
    );
    let config = ValidatedConfig::try_from(Args {
        serve_path: tmpdir.path().to_path_buf(),
        ..args
    })
    .unwrap();
    assert!(config.serve_path.is_absolute());
    assert!(config.serve_path.is_dir());
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
    assert!(xcss::config::read_private_file(file.path()).is_ok());
    for mode in [0o400, 0o440, 0o640, 0o644] {
        std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(mode)).unwrap();
        assert!(xcss::config::read_private_file(file.path()).is_err());
    }
    make_config_private(file.path());
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o750)).unwrap();
    assert!(xcss::config::read_private_file(file.path()).is_err());
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
