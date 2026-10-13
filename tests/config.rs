#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/utils.rs"]
mod utils;

use assert_cmd::prelude::*;
use assert_fs::TempDir;
use fixtures::{Error, TEST_PASSWORD, TestServer, read_bound_url, tmpdir, with_new_upload_headers};
use predicates::str::contains;
use reqwest::Method;
use rstest::rstest;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use xczs::args::{Args, build_cli};

#[rstest]
fn use_config_file(tmpdir: TempDir) -> Result<(), Error> {
    let config_dir = TempDir::new()?;
    let config_path = config_dir.path().join("config.json");
    write_private_config(&config_path, std::fs::read(get_config_path())?)?;
    let config_path = config_path.display().to_string();
    let state_dir = private_state_dir()?;
    let mut command = Command::new(assert_cmd::cargo::cargo_bin!());
    command
        .arg("run")
        .arg("--development")
        .arg("--serve-path")
        .arg(tmpdir.path())
        .args(["--bind", "127.0.0.1:0"])
        .args(["--min-free-space", "0"])
        .arg("--data-dir")
        .arg(state_dir.path())
        .args(["--config", &config_path]);
    command.env("XCZS_TEST_INITIAL_USERNAME", "user");
    fixtures::initialize_test_instance(&command);
    let mut child = command.stdout(Stdio::piped()).spawn()?;

    let port = read_bound_url(&mut child)?
        .port()
        .ok_or("Printed URL has no port")?;
    let server = TestServer::new(port, tmpdir, child, false);

    let unauthenticated = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?
        .get(format!("http://localhost:{port}/index.html"))
        .header("accept", "text/html")
        .send()?;
    assert_eq!(unauthenticated.status(), 303);

    let session = server.login("user", TEST_PASSWORD)?;
    let url = format!("http://localhost:{port}/index.html");
    let resp = server.get_with(&session, &url)?;
    assert_eq!(resp.text()?, "This is index.html");

    let url = format!("http://localhost:{port}/");
    let resp = server.get_with(&session, &url)?;
    let paths = server.paths_from_page_with(&session, resp)?;
    assert!(paths.contains("dir1/"));
    assert!(paths.contains("dir2/"));
    assert!(paths.contains("test.txt"));

    let url = format!("http://localhost:{port}/dir1/upload.txt");
    let resp = with_new_upload_headers(
        server.request_with(&session, Method::PUT, &url),
        "Hello".len() as u64,
    )
    .body("Hello")
    .send()?;
    assert_eq!(resp.status(), 201);

    Ok(())
}

#[rstest]
#[case("unexpected-setting")]
fn unknown_config_field_is_rejected(tmpdir: TempDir, #[case] field: &str) -> Result<(), Error> {
    let config_path = tmpdir.path().join("unknown-field.json");
    write_private_config(
        &config_path,
        serde_json::to_vec(&serde_json::json!({field:true}))?,
    )?;

    Command::new(assert_cmd::cargo::cargo_bin!())
        .arg("run")
        .arg("--serve-path")
        .arg(tmpdir.path())
        .args([
            "--config",
            config_path.to_str().expect("UTF-8 test path"),
            "--json",
        ])
        .assert()
        .failure()
        .stdout(contains("UNKNOWN_FIELD"))
        .stdout(contains("contract_violation"));

    Ok(())
}

#[rstest]
fn unknown_json_log_variable_is_rejected(tmpdir: TempDir) -> Result<(), Error> {
    let state_dir = private_state_dir()?;
    let config_path = tmpdir.path().join("unknown-log-variable.json");
    write_private_config(
        &config_path,
        serde_json::to_vec(
            &serde_json::json!({"data_dir":state_dir.path(),"log_format":"$stauts"}),
        )?,
    )?;
    let matches = build_cli().try_get_matches_from([
        "xczs",
        "run",
        "--serve-path",
        tmpdir.path().to_str().expect("UTF-8 test path"),
        "--config",
        config_path.to_str().expect("UTF-8 config path"),
    ])?;

    let error = Args::parse(matches).expect_err("unknown JSON log variable was accepted");
    let violation = error
        .downcast_ref::<xcss::config::ConfigError>()
        .expect("malformed file log format must use the configuration contract");
    assert_eq!(violation.reason, xcss::config::Reason::InvalidValue);
    assert_eq!(violation.source, xcss::config::ConfigSource::File);
    assert_eq!(violation.path, "/log_format");
    assert!(!error.to_string().contains("$stauts"));
    Ok(())
}

#[rstest]
fn legacy_json_bind_array_is_rejected(tmpdir: TempDir) -> Result<(), Error> {
    let state_dir = private_state_dir()?;
    let config_path = tmpdir.path().join("duplicate-bind.json");
    write_private_config(
        &config_path,
        serde_json::to_vec(
            &serde_json::json!({"data_dir":state_dir.path(),"bind":["127.0.0.1","127.0.0.1"]}),
        )?,
    )?;
    let matches = build_cli().try_get_matches_from([
        "xczs",
        "run",
        "--serve-path",
        tmpdir.path().to_str().expect("UTF-8 test path"),
        "--config",
        config_path.to_str().expect("UTF-8 config path"),
    ])?;

    let error = Args::parse(matches).expect_err("legacy JSON bind array was accepted");
    let violation = error
        .downcast_ref::<xcss::config::ConfigError>()
        .expect("array bind must use the configuration contract");
    assert_eq!(violation.reason, xcss::config::Reason::TypeMismatch);
    assert_eq!(violation.source, xcss::config::ConfigSource::File);
    assert_eq!(violation.path, "/bind");
    assert!(!error.to_string().contains("127.0.0.1"));
    Ok(())
}

#[rstest]
fn deployment_json_example_parses(tmpdir: TempDir) -> Result<(), Error> {
    const STATE_DIR: &str = "/var/lib/isarmg/xczs";

    let example = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config/config.json.example");
    let template = std::fs::read_to_string(example)?;
    assert_eq!(
        template.matches(STATE_DIR).count(),
        1,
        "deployment state directory path must occur exactly once"
    );

    let state_root = private_state_dir()?;
    let state_dir = state_root.path().join("state dir & # \"quoted\"");
    std::fs::create_dir(&state_dir)?;
    std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o700))?;
    let config_dir = TempDir::new()?;
    let mut rendered: serde_json::Value = serde_json::from_str(&template)?;
    rendered["data_dir"] = serde_json::json!(state_dir);
    let rendered = serde_json::to_vec(&rendered)?;
    let config = config_dir.path().join("xczs.json");
    write_private_config(&config, rendered)?;
    let matches = build_cli().try_get_matches_from([
        "xczs",
        "run",
        "--serve-path",
        tmpdir.path().to_str().expect("UTF-8 test path"),
        "--config",
        config.to_str().expect("UTF-8 test path"),
    ])?;
    let args = Args::parse(matches)?;

    assert_eq!(args.serve_path, std::fs::canonicalize(tmpdir.path())?);
    assert_eq!(args.state_dir.as_deref(), Some(state_dir.as_path()));
    assert_eq!(args.bind, "127.0.0.1:5000".parse().unwrap());
    assert!(!args.development);
    Ok(())
}

#[rstest]
fn cli_state_dir_overrides_json_state_dir(tmpdir: TempDir) -> Result<(), Error> {
    let json_state_dir = private_state_dir()?;
    let cli_state_dir = private_state_dir()?;
    let config_dir = TempDir::new()?;
    let config = config_dir.path().join("state-dir.json");
    write_private_config(
        &config,
        serde_json::to_vec(&serde_json::json!({"data_dir":json_state_dir.path()}))?,
    )?;

    let json_matches = build_cli().try_get_matches_from([
        "xczs",
        "run",
        "--serve-path",
        tmpdir.path().to_str().expect("UTF-8 test path"),
        "--config",
        config.to_str().expect("UTF-8 test path"),
    ])?;
    let json_args = Args::parse(json_matches)?;
    assert_eq!(json_args.state_dir.as_deref(), Some(json_state_dir.path()));

    let cli_matches = build_cli().try_get_matches_from([
        "xczs",
        "run",
        "--serve-path",
        tmpdir.path().to_str().expect("UTF-8 test path"),
        "--config",
        config.to_str().expect("UTF-8 test path"),
        "--data-dir",
        cli_state_dir.path().to_str().expect("UTF-8 test path"),
    ])?;
    let cli_args = Args::parse(cli_matches)?;
    assert_eq!(cli_args.state_dir.as_deref(), Some(cli_state_dir.path()));
    Ok(())
}

#[test]
fn deployment_service_uses_the_single_current_layout() -> Result<(), Error> {
    let service = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("systemd/xczs.service");
    let service = std::fs::read_to_string(service)?;
    assert!(service.lines().any(|line| line == "User=isarmg-xczs"));
    assert!(service.lines().any(|line| line == "Group=isarmg-xczs"));
    assert!(service.contains("/opt/isarmg/xczs/bin/xczs"));
    assert!(service.contains("/etc/isarmg/xczs/config.json"));
    Ok(())
}

#[rstest]
#[case("")]
#[case("-journal")]
fn state_dir_rejects_configuration_file_collisions(
    tmpdir: TempDir,
    #[case] suffix: &str,
) -> Result<(), Error> {
    let state_dir = private_state_dir()?;
    let state_db = state_dir.path().join("state.sqlite3");
    let mut config_path = state_db.as_os_str().to_os_string();
    config_path.push(suffix);
    let config_path = PathBuf::from(config_path);
    write_private_config(
        &config_path,
        serde_json::to_vec(&serde_json::json!({"data_dir":state_dir.path()}))?,
    )?;
    let matches = build_cli().try_get_matches_from([
        "xczs",
        "run",
        "--serve-path",
        tmpdir.path().to_str().expect("UTF-8 test path"),
        "--config",
        config_path.to_str().expect("UTF-8 config path"),
    ])?;

    let error = Args::parse(matches).expect_err("SQLite/config path collision was accepted");
    assert!(
        error
            .to_string()
            .contains("conflicts with SQLite state database"),
        "unexpected error: {error:#}"
    );
    Ok(())
}

fn get_config_path() -> PathBuf {
    let mut path = std::env::current_dir().expect("Failed to get current directory");
    path.push("tests");
    path.push("data");
    path.push("config.json");
    path
}

fn private_state_dir() -> Result<TempDir, Error> {
    let state_dir = TempDir::new()?;
    std::fs::set_permissions(state_dir.path(), std::fs::Permissions::from_mode(0o700))?;
    Ok(state_dir)
}

fn write_private_config(path: &Path, contents: impl AsRef<[u8]>) -> Result<(), Error> {
    std::fs::set_permissions(
        path.parent().unwrap(),
        std::fs::Permissions::from_mode(0o700),
    )?;
    std::fs::write(path, contents)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    if let Err(error) = rustix::fs::removexattr(path, "system.posix_acl_access")
        && error != rustix::io::Errno::NODATA
        && error != rustix::io::Errno::NOTSUP
    {
        return Err(std::io::Error::from(error).into());
    }
    Ok(())
}
