#[path = "support/fixtures.rs"]
mod fixtures;

use fixtures::{Error, TestServer, server, with_new_upload_headers};
use reqwest::header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use reqwest::{Method, StatusCode};
use rstest::rstest;
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use xczs::utils::encode_hex;

#[test]
fn production_directory_override_fails_before_persistent_state_changes() -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt;
    let root = assert_fs::TempDir::new()?;
    let state = assert_fs::TempDir::new()?;
    std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700))?;
    let (mut command, config) = fixtures::xczs_command(&[fixtures::TEST_ACCOUNT]);
    command
        .env_remove("XCZS_DEV_WEB_DIR")
        .arg("--serve-path")
        .arg(root.path())
        .args(["--bind", "127.0.0.1:0", "--min-free-space", "0"])
        .arg("--data-dir")
        .arg(state.path());
    fixtures::initialize_test_instance(&command);
    assert!(state.path().join("state.sqlite3").is_file());
    assert!(state.path().join("tags.db").is_file());
    assert!(
        state
            .path()
            .join("administrator/administrator-accounts.json")
            .is_file()
    );
    let before_state = persisted_tree(state.path())?;
    let before_root = persisted_tree(root.path())?;
    let before_config = persisted_tree(config.path().parent().unwrap())?;
    command
        .arg("run")
        .arg("--json")
        .env("XCZS_DEV_WEB_DIR", root.path());
    let output = command.output()?;
    assert_eq!(output.status.code(), Some(1));
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(envelope["code"], "contract_violation");
    assert_eq!(
        envelope["details"]["violations"],
        json!([{
            "reason": "INVALID_VALUE",
            "path": "/XCZS_DEV_WEB_DIR",
            "source": "environment"
        }])
    );
    assert!(output.stderr.is_empty());
    assert_eq!(persisted_tree(state.path())?, before_state);
    assert_eq!(persisted_tree(root.path())?, before_root);
    assert_eq!(
        persisted_tree(config.path().parent().unwrap())?,
        before_config
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains(root.path().to_str().unwrap()));
    if env!("XCZS_BUILD_GIT_SHA") != "unbound" {
        // A source-bound executable must also refuse the override when its
        // explicit development flag and loopback bind are otherwise valid.
        command.arg("--development");
        let output = command.output()?;
        assert_eq!(output.status.code(), Some(1));
        let envelope: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(envelope["code"], "contract_violation");
        assert_eq!(
            envelope["details"]["violations"],
            json!([{
                "reason": "INVALID_VALUE",
                "path": "/XCZS_DEV_WEB_DIR",
                "source": "environment"
            }])
        );
        assert!(output.stderr.is_empty());
        assert_eq!(persisted_tree(state.path())?, before_state);
        assert_eq!(persisted_tree(root.path())?, before_root);
        assert_eq!(
            persisted_tree(config.path().parent().unwrap())?,
            before_config
        );
    }
    // Explicit initialization must reject the same selection before creating
    // even a data-directory leaf, irrespective of otherwise valid CLI values.
    let fresh_parent = assert_fs::TempDir::new()?;
    let fresh_state = fresh_parent.path().join("new-state");
    let (mut initialize, _config) = fixtures::xczs_command(&[fixtures::TEST_ACCOUNT]);
    initialize
        .arg("init")
        .args(["--json", "--serve-path"])
        .arg(root.path())
        .args(["--data-dir"])
        .arg(&fresh_state)
        .env("XCZS_DEV_WEB_DIR", root.path());
    let output = initialize.output()?;
    assert_eq!(output.status.code(), Some(1));
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(envelope["code"], "contract_violation");
    assert_eq!(
        envelope["details"]["violations"][0]["reason"],
        "INVALID_VALUE"
    );
    assert_eq!(
        envelope["details"]["violations"][0]["source"],
        "environment"
    );
    assert!(!fresh_state.exists());
    assert_eq!(std::fs::read_dir(fresh_parent.path())?.count(), 0);
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct PersistentEntry {
    device: u64,
    inode: u64,
    mode: u32,
    links: u64,
    bytes: Option<Vec<u8>>,
}

fn persisted_tree(
    root: &std::path::Path,
) -> Result<std::collections::BTreeMap<std::path::PathBuf, PersistentEntry>, Error> {
    use std::os::unix::fs::MetadataExt;
    let mut pending = vec![root.to_path_buf()];
    let mut entries = std::collections::BTreeMap::new();
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path)?;
        assert!(!metadata.is_symlink());
        if metadata.is_dir() {
            pending.extend(std::fs::read_dir(&path)?.map(|entry| entry.unwrap().path()));
        }
        entries.insert(
            path.strip_prefix(root)?.to_path_buf(),
            PersistentEntry {
                device: metadata.dev(),
                inode: metadata.ino(),
                mode: metadata.mode(),
                links: metadata.nlink(),
                bytes: metadata
                    .is_file()
                    .then(|| std::fs::read(path))
                    .transpose()?,
            },
        );
    }
    Ok(entries)
}

const LONG_LIVED_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";
const PRIVATE_NO_STORE: &str = "private, no-store";

#[rstest]
fn embedded_assets_are_content_addressed_and_cacheable(server: TestServer) -> Result<(), Error> {
    verify_embedded_assets(&server)
}

fn verify_embedded_assets(server: &TestServer) -> Result<(), Error> {
    let page = server.get(server.url())?.error_for_status()?.text()?;
    let index_js = extract_asset_url(&page, "src", "index.js")?;
    let index_css = extract_asset_url(&page, "href", "index.css")?;
    let favicon = extract_asset_url(&page, "href", "favicon.ico")?;

    let asset_prefix = shared_asset_prefix([&index_js, &index_css, &favicon])?;
    let advertised_digest = asset_prefix
        .strip_prefix("/__xczs_assets_")
        .and_then(|value| value.strip_suffix('/'))
        .ok_or("Embedded asset prefix has an unexpected form")?;

    let manifest = xczs::server::web_assets_manifest()?;
    xcss::web_assets::verify_manifest(manifest, advertised_digest)?;
    let inventory: xcss::web_assets::AssetManifest = serde_json::from_str(manifest)?;
    assert!(inventory.files.iter().any(|file| file.path == "index.js"));
    assert!(
        inventory
            .files
            .iter()
            .any(|file| file.path == "dist/platform.js")
    );
    assert!(
        inventory
            .files
            .iter()
            .any(|file| file.path.ends_with(".woff2"))
    );
    assert!(
        !inventory
            .files
            .iter()
            .any(|file| file.path.ends_with(".d.ts"))
    );
    for record in inventory.files {
        let path = format!("{asset_prefix}{}", record.path);
        let url = server.url().join(&path)?;
        let response = server.get(url.clone())?;
        assert_eq!(response.status(), StatusCode::OK, "asset={path}");
        assert_eq!(
            response.headers()[CONTENT_TYPE],
            record.content_type,
            "asset={path}"
        );
        assert_eq!(
            response.headers()[CACHE_CONTROL],
            LONG_LIVED_CACHE_CONTROL,
            "asset={path}"
        );
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        let etag = response.headers()[ETAG].clone();
        let contents = response.bytes()?;
        assert_eq!(contents.len() as u64, record.size, "asset={path}");
        assert_eq!(
            encode_hex(Sha256::digest(&contents)),
            record.sha256,
            "asset={path}"
        );
        let head = server.request(Method::HEAD, url.clone()).send()?;
        assert_eq!(head.status(), StatusCode::OK, "asset={path}");
        assert_eq!(head.headers()[CONTENT_LENGTH], record.size.to_string());
        assert_eq!(head.headers()[ETAG], etag);
        assert!(head.bytes()?.is_empty());
        let unchanged = server
            .request(Method::GET, url)
            .header(IF_NONE_MATCH, etag.clone())
            .send()?;
        assert_eq!(unchanged.status(), StatusCode::NOT_MODIFIED, "asset={path}");
        assert_eq!(unchanged.headers()[ETAG], etag);
        assert_eq!(unchanged.headers()[CACHE_CONTROL], LONG_LIVED_CACHE_CONTROL);
        assert!(unchanged.bytes()?.is_empty());
    }

    let missing_path = format!("{asset_prefix}missing.js");
    let missing = server.get(server.url().join(&missing_path)?)?;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        missing
            .headers()
            .get(CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some(PRIVATE_NO_STORE)
    );
    assert_eq!(missing.text()?, "Not Found");

    let reserved_component = asset_prefix
        .strip_prefix('/')
        .and_then(|value| value.strip_suffix('/'))
        .ok_or("Embedded asset URL has an invalid reserved component")?;
    let upload = with_new_upload_headers(
        server.request(
            Method::PUT,
            server.url().join(&format!("{asset_prefix}user-file.txt"))?,
        ),
        7,
    )
    .body("blocked")
    .send()?;
    assert_eq!(upload.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(upload.headers()["allow"], "GET, HEAD");

    let mkdir = server
        .request(Method::POST, server.url().join("__xczs__/api/mkdir")?)
        .header(CONTENT_TYPE, "application/json")
        .header("X-Xczs-Operation-Id", Uuid::new_v4().to_string())
        .body(json!({"path": format!("/{reserved_component}/directory")}).to_string())
        .send()?;
    assert_eq!(mkdir.status(), StatusCode::BAD_REQUEST);
    assert!(!server.path().join(reserved_component).exists());
    Ok(())
}

fn extract_asset_url(page: &str, attribute: &str, filename: &str) -> Result<String, Error> {
    let marker = format!(r#"{attribute}=""#);
    let expected_suffix = format!("/{filename}");
    let mut found = None;

    for (start, _) in page.match_indices(&marker) {
        let value = &page[start + marker.len()..];
        let Some(end) = value.find('"') else {
            return Err(format!("The {attribute} attribute is missing its closing quote").into());
        };
        let value = &value[..end];
        if !value.ends_with(&expected_suffix) {
            continue;
        }
        if found.replace(value.to_string()).is_some() {
            return Err(format!("Directory page contains duplicate {filename} URLs").into());
        }
    }

    let path = found.ok_or_else(|| format!("Directory page is missing the {filename} URL"))?;
    if !path.starts_with('/') {
        return Err(format!("Embedded asset URL is not absolute: {path}").into());
    }
    Ok(path)
}

fn shared_asset_prefix(paths: [&str; 3]) -> Result<String, Error> {
    let filenames = ["index.js", "index.css", "favicon.ico"];
    let mut shared_prefix = None;

    for (path, filename) in paths.into_iter().zip(filenames) {
        let prefix = path
            .strip_suffix(filename)
            .ok_or_else(|| format!("Embedded asset URL does not end with {filename}: {path}"))?;
        if let Some(shared_prefix) = shared_prefix {
            if shared_prefix != prefix {
                return Err("Directory page assets do not share one content digest prefix".into());
            }
        } else {
            shared_prefix = Some(prefix);
        }
    }

    let shared_prefix = shared_prefix.ok_or("Directory page does not contain embedded assets")?;
    let digest = shared_prefix
        .strip_prefix("/__xczs_assets_")
        .and_then(|value| value.strip_suffix('/'))
        .ok_or_else(|| format!("Unexpected embedded asset prefix: {shared_prefix}"))?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(format!("Embedded asset digest is not lowercase SHA-256: {digest}").into());
    }

    Ok(shared_prefix.to_string())
}
