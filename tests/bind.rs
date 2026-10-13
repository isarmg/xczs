#[path = "support/fixtures.rs"]
mod fixtures;

use fixtures::{
    ADMIN_ACCOUNT, Error, TEST_ACCOUNT, TEST_PASSWORD, TEST_USER, TestServer, production_server,
    read_bound_url, server, tmpdir, xczs_command,
};

use assert_cmd::prelude::*;
use assert_fs::fixture::TempDir;
use reqwest::blocking::Client;
use rstest::rstest;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

fn private_state_dir() -> Result<TempDir, Error> {
    let state_dir = TempDir::new()?;
    std::fs::set_permissions(state_dir.path(), std::fs::Permissions::from_mode(0o700))?;
    Ok(state_dir)
}
#[rstest]
#[case(&["-b", "20.205.243.166:0"])]
fn bind_fails(tmpdir: TempDir, #[case] args: &[&str]) -> Result<(), Error> {
    let state_dir = private_state_dir()?;
    let (mut command, _auth_config) = xczs_command(&[TEST_ACCOUNT]);
    command.arg("run");
    command
        .arg("--serve-path")
        .arg(tmpdir.path())
        .arg("--data-dir")
        .arg(state_dir.path())
        .args(args);
    fixtures::initialize_test_instance(&command);
    command
        .assert()
        .stderr(predicates::str::contains("Failed to bind"))
        .failure();

    Ok(())
}

#[rstest]
fn invalid_multiple_bind_input_does_not_initialize_persistent_state(
    tmpdir: TempDir,
) -> Result<(), Error> {
    let state_dir = private_state_dir()?;
    let (mut command, _auth_config) = xczs_command(&[TEST_ACCOUNT]);
    command.arg("run");
    command
        .arg("--serve-path")
        .arg(tmpdir.path())
        .arg("--data-dir")
        .arg(state_dir.path())
        .args(["--bind", "127.0.0.1:0", "--bind", "20.205.243.166:0"])
        .assert()
        .stderr(predicates::str::contains("invalid_cli_input"))
        .failure();

    assert!(
        !state_dir.path().join("state.sqlite3").exists(),
        "invalid listener input must not initialize persistent state"
    );
    Ok(())
}

#[rstest]
#[case("not-an-ip-address")]
#[case("localhost")]
fn non_ip_bind_is_rejected(tmpdir: TempDir, #[case] bind: &str) -> Result<(), Error> {
    let (mut command, _auth_config) = xczs_command(&[TEST_ACCOUNT]);
    command.arg("run");
    command
        .arg("--serve-path")
        .arg(tmpdir.path())
        .args(["--bind", bind])
        .assert()
        .stderr(predicates::str::contains("invalid_cli_input"))
        .failure();

    Ok(())
}

#[rstest]
#[case(server(&[] as &[&str], &[TEST_ACCOUNT]), true, false)]
#[case(production_server(&["-b", "0.0.0.0:0"], &[ADMIN_ACCOUNT]), true, false)]
#[case(server(&["-b", "[::1]:0"], &[ADMIN_ACCOUNT]), false, true)]
fn bind_ipv4_ipv6(
    #[case] server: TestServer,
    #[case] bind_ipv4: bool,
    #[case] bind_ipv6: bool,
) -> Result<(), Error> {
    assert_eq!(
        reqwest::blocking::get(format!("http://127.0.0.1:{}", server.port()).as_str()).is_ok(),
        bind_ipv4
    );
    assert_eq!(
        reqwest::blocking::get(format!("http://[::1]:{}", server.port()).as_str()).is_ok(),
        bind_ipv6
    );

    Ok(())
}

// These listener-only tests use a valid non-default account so the fixture
// does not spend the sole connection permit on its automatic setup login.
#[rstest]
fn sequential_requests_release_the_single_connection_permit(
    #[with(&[
        "--bind",
        "127.0.0.1:0",
        "--max-connections",
        "1",
    ], &[ADMIN_ACCOUNT])]
    server: TestServer,
) -> Result<(), Error> {
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()?;
    for address in ["127.0.0.1", "127.0.0.1"] {
        let response = client
            .get(format!("http://{address}:{}/healthz", server.port()))
            .header("connection", "close")
            .send()?;
        assert_eq!(response.status(), 204);
    }
    Ok(())
}

#[rstest]
fn connection_limit_bounds_userspace_sockets(
    #[with(&[
        "--bind",
        "127.0.0.1:0",
        "--max-connections",
        "1",
    ], &[ADMIN_ACCOUNT])]
    server: TestServer,
) -> Result<(), Error> {
    let pid = server.process_id();
    let baseline = socket_fd_count(pid)?;
    assert!(baseline >= 1, "the fixture did not expose its listener");

    let held = TcpStream::connect(("127.0.0.1", server.port()))?;
    wait_for_socket_fd_count(pid, baseline + 1)?;

    let mut queued_first = TcpStream::connect(("127.0.0.1", server.port()))?;
    let mut queued_second = TcpStream::connect(("127.0.0.1", server.port()))?;
    sleep(Duration::from_millis(200));
    assert_eq!(
        socket_fd_count(pid)?,
        baseline + 1,
        "listeners accepted sockets that did not own a connection permit"
    );

    for stream in [&mut queued_first, &mut queued_second] {
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.write_all(
            format!(
                "GET /healthz HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
                server.port()
            )
            .as_bytes(),
        )?;
    }
    drop(held);

    for stream in [&mut queued_first, &mut queued_second] {
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        assert!(
            response.starts_with("HTTP/1.1 204 "),
            "queued connection returned an unexpected response: {response:?}"
        );
    }
    Ok(())
}

fn socket_fd_count(pid: u32) -> Result<usize, Error> {
    let mut count = 0;
    for entry in fs::read_dir(format!("/proc/{pid}/fd"))? {
        let target = match fs::read_link(entry?.path()) {
            Ok(target) => target,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err.into()),
        };
        if target.to_string_lossy().starts_with("socket:[") {
            count += 1;
        }
    }
    Ok(count)
}

fn wait_for_socket_fd_count(pid: u32, expected: usize) -> Result<(), Error> {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if socket_fd_count(pid)? == expected {
            return Ok(());
        }
        sleep(Duration::from_millis(10));
    }
    Err(format!(
        "server socket fd count did not reach {expected}; observed {}",
        socket_fd_count(pid)?
    )
    .into())
}

#[rstest]
fn validate_printed_url(tmpdir: TempDir) -> Result<(), Error> {
    let state_dir = private_state_dir()?;
    let (mut command, _auth_config) = xczs_command(&[TEST_ACCOUNT]);
    command.arg("run");
    command
        .arg("--development")
        .arg("--serve-path")
        .arg(tmpdir.path())
        .args(["--bind", "127.0.0.1:0"])
        .arg("--data-dir")
        .arg(state_dir.path())
        .stdout(Stdio::piped());
    fixtures::initialize_test_instance(&command);
    let mut child = command.spawn()?;

    let printed_url = read_bound_url(&mut child)?;
    let port = printed_url.port().ok_or("Printed URL has no port")?;
    assert_eq!(printed_url.path(), "/");
    let server = TestServer::new(port, tmpdir, child, false);
    let session = server.login(TEST_USER, TEST_PASSWORD)?;
    server
        .get_with(&session, server.url())?
        .error_for_status()?;

    Ok(())
}

#[rstest]
fn closed_stdout_does_not_abort_the_server(tmpdir: TempDir) -> Result<(), Error> {
    let port_probe = TcpListener::bind(("127.0.0.1", 0))?;
    let port = port_probe.local_addr()?.port();
    drop(port_probe);

    let state_dir = private_state_dir()?;
    let (mut command, _auth_config) = xczs_command(&[TEST_ACCOUNT]);
    command.arg("run");
    command
        .arg("--serve-path")
        .arg(tmpdir.path())
        .arg("--bind")
        .arg(format!("127.0.0.1:{port}"))
        .arg("--data-dir")
        .arg(state_dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    fixtures::initialize_test_instance(&command);
    let mut child = command.spawn()?;
    drop(child.stdout.take().ok_or("Missing child stdout")?);

    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(200))
        .build()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if client
            .get(format!("http://127.0.0.1:{port}/healthz"))
            .send()
            .is_ok_and(|response| response.status() == 204)
        {
            break;
        }
        if let Some(status) = child.try_wait()? {
            return Err(format!("server exited after stdout closed: {status}").into());
        }
        if Instant::now() >= deadline {
            return Err("server did not become healthy after stdout closed".into());
        }
        sleep(Duration::from_millis(20));
    }

    let signal = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()?;
    if !signal.success() {
        return Err("Failed to send SIGTERM to test server".into());
    }
    let status = child.wait()?;
    assert!(status.success(), "server did not stop cleanly: {status}");
    Ok(())
}
