#[path = "support/fixtures.rs"]
mod fixtures;

use fixtures::{Error, TEST_PASSWORD, TestServer, USER_ACCOUNT, server};
use rstest::rstest;

const HEALTH_CHECK_PATH: &str = "healthz";
const HEALTH_CHECK_RESPONSE: &str = "";
const READINESS_CHECK_PATH: &str = "readyz";
const READINESS_CHECK_RESPONSE: &str = r#"{"ready":true}"#;

fn assert_no_store(response: &reqwest::blocking::Response) {
    let cache_control = response
        .headers()
        .get("cache-control")
        .expect("health responses must define Cache-Control")
        .to_str()
        .expect("Cache-Control must be ASCII");
    assert!(
        cache_control
            .split(',')
            .any(|directive| directive.trim().eq_ignore_ascii_case("no-store")),
        "Cache-Control must contain no-store, got {cache_control:?}"
    );
}

#[rstest]
fn normal_health(server: TestServer) -> Result<(), Error> {
    let resp = server.get(format!("{}{HEALTH_CHECK_PATH}", server.url()))?;
    assert_no_store(&resp);
    assert_eq!(resp.text()?, HEALTH_CHECK_RESPONSE);
    Ok(())
}

#[rstest]
fn removed_health_paths_do_not_alias_platform_health(server: TestServer) -> Result<(), Error> {
    for path in ["__xczs__/health", "__xczs__/ready"] {
        let response = server.get(server.url().join(path)?)?;
        assert_eq!(response.status(), 404);
        assert!(!response.headers().contains_key("location"));
        assert!(!response.text()?.contains("\"ready\""));
    }
    Ok(())
}

#[rstest]
fn auth_health(
    #[with(serde_json::json!({}), &[USER_ACCOUNT])] server: TestServer,
) -> Result<(), Error> {
    let url = format!("{}{HEALTH_CHECK_PATH}", server.url());
    let resp = server.raw_request(reqwest::Method::GET, &url).send()?;
    assert_eq!(resp.status(), 204);
    assert_eq!(resp.text()?, HEALTH_CHECK_RESPONSE);

    let ready_url = format!("{}{READINESS_CHECK_PATH}", server.url());
    let unauthenticated = server
        .raw_request(reqwest::Method::GET, &ready_url)
        .send()?;
    assert_eq!(unauthenticated.status(), 200);
    assert_eq!(unauthenticated.text()?, READINESS_CHECK_RESPONSE);

    let session = server.login("user", TEST_PASSWORD)?;
    let resp = server.get_with(&session, &ready_url)?;
    assert_no_store(&resp);
    assert_eq!(resp.text()?, READINESS_CHECK_RESPONSE);

    let head = server
        .request_with(&session, reqwest::Method::HEAD, &ready_url)
        .send()?;
    assert_eq!(head.status(), 200);
    assert_no_store(&head);
    assert_eq!(head.text()?, "");
    Ok(())
}

#[rstest]
fn health_supports_head_and_rejects_other_methods_with_allow(
    server: TestServer,
) -> Result<(), Error> {
    let url = format!("{}{HEALTH_CHECK_PATH}", server.url());
    let head = server.raw_request(reqwest::Method::HEAD, &url).send()?;
    assert_eq!(head.status(), 204);
    assert_eq!(head.text()?, "");

    let rejected = server.raw_request(reqwest::Method::POST, &url).send()?;
    assert_eq!(rejected.status(), 405);
    assert_eq!(
        rejected.headers()["allow"]
            .to_str()?
            .split(',')
            .map(str::trim)
            .collect::<Vec<_>>(),
        vec!["GET", "HEAD"]
    );
    Ok(())
}

#[rstest]
fn readiness_reports_insufficient_protected_disk_space(
    #[with(serde_json::json!({"min_free_space": 18446744073709551615u64}))] server: TestServer,
) -> Result<(), Error> {
    let health = server.get(format!("{}{HEALTH_CHECK_PATH}", server.url()))?;
    assert_eq!(health.status(), 204);

    let readiness = server.get(format!("{}{READINESS_CHECK_PATH}", server.url()))?;
    assert_eq!(readiness.status(), 503);
    assert_no_store(&readiness);
    assert!(readiness.headers().get("retry-after").is_none());
    assert_eq!(readiness.text()?, r#"{"ready":false}"#);
    Ok(())
}
