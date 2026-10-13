use assert_cmd::prelude::*;
use std::process::Command;

#[test]
fn runtime_rejects_retired_maintenance_commands() {
    for command in [
        "run",
        "init",
        "check",
        "status",
        "config",
        "admin",
        "release-identity",
        "web-assets",
    ] {
        Command::new(assert_cmd::cargo::cargo_bin!())
            .arg(command)
            .assert()
            .failure();
    }
}
