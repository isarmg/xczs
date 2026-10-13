//! Run cli with different args, not starting a server

#[path = "support/fixtures.rs"]
mod fixtures;

use assert_cmd::prelude::*;
use fixtures::Error;
use predicates::str::contains;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
/// Show help and exit.
fn help_shows() -> Result<(), Error> {
    Command::new(assert_cmd::cargo::cargo_bin!())
        .arg("-h")
        .assert()
        .success();
    Ok(())
}

#[test]
fn version_includes_source_revision() -> Result<(), Error> {
    let revision = env!("XCZS_BUILD_GIT_SHA");
    assert!(
        revision == "unbound"
            || ((7..=64).contains(&revision.len())
                && revision.bytes().all(|byte| byte.is_ascii_hexdigit()))
    );
    let foundation = env!("XCSS_REVISION");
    assert_eq!(foundation.len(), 40);
    assert!(foundation.bytes().all(|byte| byte.is_ascii_hexdigit()));
    Command::new(assert_cmd::cargo::cargo_bin!())
        .arg("--version")
        .assert()
        .success()
        .stdout(format!(
            "xczs {} (git {revision}) xcss={foundation}\n",
            env!("CARGO_PKG_VERSION")
        ));
    Ok(())
}

#[test]
fn run_requires_initialized_state_without_creating_accounts() -> Result<(), Error> {
    let shared_root = assert_fs::TempDir::new()?;
    let state_dir = assert_fs::TempDir::new()?;
    std::fs::set_permissions(state_dir.path(), std::fs::Permissions::from_mode(0o700))?;
    Command::new(assert_cmd::cargo::cargo_bin!())
        .args(["run", "--serve-path"])
        .arg(shared_root.path())
        .arg("--data-dir")
        .arg(state_dir.path())
        .env_remove("XCZS_AUTH")
        .env_remove("XCZS_DEV_WEB_DIR")
        .assert()
        .code(1)
        .stderr(contains("current_state_invalid"));
    assert_eq!(std::fs::read_dir(state_dir.path())?.count(), 0);
    assert_eq!(std::fs::read_dir(shared_root.path())?.count(), 0);
    Ok(())
}

#[test]
fn unknown_option_is_rejected() -> Result<(), Error> {
    let output = Command::new(assert_cmd::cargo::cargo_bin!())
        .args([
            "run",
            "--json",
            "--definitely-unknown-option",
            "untrusted-cli-value",
        ])
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    let envelope: xcss::server_cli::ErrorEnvelope = serde_json::from_slice(&output.stdout)?;
    assert_eq!(envelope.code.as_str(), "invalid_cli_input");
    assert_eq!(
        envelope.message,
        "Command arguments do not satisfy the current CLI contract; use --help."
    );
    assert!(!envelope.retryable);
    assert!(envelope.details.is_empty());
    assert!(output.stderr.is_empty());
    let machine_output = String::from_utf8(output.stdout)?;
    assert!(!machine_output.contains("--definitely-unknown-option"));
    assert!(!machine_output.contains("untrusted-cli-value"));
    Ok(())
}

#[test]
fn no_implicit_run_without_a_subcommand() -> Result<(), Error> {
    let output = Command::new(assert_cmd::cargo::cargo_bin!())
        .arg("--json")
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    let envelope: xcss::server_cli::ErrorEnvelope = serde_json::from_slice(&output.stdout)?;
    assert_eq!(envelope.code.as_str(), "invalid_cli_input");
    Ok(())
}
