use std::{path::Path, process::Command};

fn main() {
    println!("cargo:rerun-if-env-changed=XSSI_BUILD_METADATA_DIR");
    let tagging = include_str!("schema/tagging.sql");
    let tagging_fingerprint =
        xcss::sqlite::fingerprint_trusted_ddl(&[tagging]).expect("current tag schema fingerprint");
    println!("cargo:rustc-env=XCZS_TAG_SCHEMA_SHA256={tagging_fingerprint}");
    println!("cargo:rerun-if-changed=schema/tagging.sql");

    let schema = format!(
        "{}\n{}",
        include_str!("schema/product.sql"),
        xcss::admin_sqlite::ADMIN_PERSISTENT_DDL
    );
    std::fs::write(
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("current_schema.sql"),
        &schema,
    )
    .expect("write combined current schema");
    let fingerprint =
        xcss::sqlite::fingerprint_trusted_ddl(&[&schema]).expect("current XCZS schema fingerprint");
    println!("cargo:rustc-env=XCZS_CURRENT_SCHEMA_SHA256={fingerprint}");
    println!("cargo:rerun-if-changed=schema/product.sql");

    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let pointer_width = std::env::var("CARGO_CFG_TARGET_POINTER_WIDTH").unwrap_or_default();
    assert!(
        target_arch == "x86_64"
            && target_env == "gnu"
            && target_os == "linux"
            && pointer_width == "64",
        "xczs server supports only x86_64-unknown-linux-gnu (requested arch={target_arch}, os={target_os}, env={target_env}, pointer_width={pointer_width})"
    );

    println!("cargo:rerun-if-env-changed=XCZS_SOURCE_REVISION");
    println!("cargo:rerun-if-env-changed=XCSS_WEB_DIST");
    emit_git_rerun_paths();
    println!("cargo:rustc-env=XCZS_SOURCE_REVISION={}", build_git_sha());
    let lockfile = std::fs::read_to_string("Cargo.lock").expect("read Cargo.lock");
    let revisions: std::collections::BTreeSet<_> = lockfile
        .lines()
        .filter_map(|line| {
            line.strip_prefix("source = \"git+https://github.com/isarmg/xcss.git?rev=")
        })
        .map(|line| {
            let (requested, locked) = line
                .trim_end_matches('"')
                .split_once('#')
                .expect("xcss source requires a locked commit");
            assert!(
                requested.len() == 40
                    && requested.bytes().all(|byte| byte.is_ascii_hexdigit())
                    && requested == locked,
                "xcss source must bind one exact full commit"
            );
            requested.to_owned()
        })
        .collect();
    let mut revisions = revisions;
    if revisions.is_empty()
        && std::env::var("XCZS_SOURCE_REVISION").is_ok_and(|value| value == "unbound")
        && std::env::var("PROFILE").is_ok_and(|profile| profile == "debug")
        && std::env::var_os("XSSI_BUILD_METADATA_DIR").is_none()
    {
        revisions.insert("unbound".into());
    }
    assert!(
        revisions.len() == 1,
        "one exact xcss Git revision is required"
    );
    println!(
        "cargo:rustc-env=XCSS_REVISION={}",
        revisions.iter().next().unwrap()
    );
    println!("cargo:rerun-if-changed=Cargo.lock");

    let root = std::env::var_os("XCSS_WEB_DIST")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/runtime-dist")
        });
    xcss::web_assets::build::generate(root)
        .expect("build the Web platform with the xcss builder before compiling Server");
    emit_offline_metadata();
}

fn emit_git_rerun_paths() {
    // A linked worktree stores a small `.git` file in the checkout and keeps
    // HEAD plus the shared references elsewhere. Ask Git for those real paths
    // instead of assuming that `.git` is a directory below the package root.
    if Path::new(".git").is_file() {
        println!("cargo:rerun-if-changed=.git");
    }
    for name in ["HEAD", "refs", "packed-refs", "reftable"] {
        if let Some(path) = git_output(&["rev-parse", "--git-path", name]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    // Shared refs in a reftable repository live below GIT_COMMON_DIR, while a
    // linked worktree's `--git-path reftable` resolves its private ref stack.
    if let Some(common_dir) = git_output(&["rev-parse", "--git-common-dir"]) {
        println!(
            "cargo:rerun-if-changed={}",
            Path::new(&common_dir).join("reftable").display()
        );
    }
}

fn build_git_sha() -> String {
    if let Ok(value) = std::env::var("XCZS_SOURCE_REVISION") {
        let value = value.trim();
        if value == "unbound" {
            assert!(
                std::env::var("PROFILE").is_ok_and(|profile| profile == "debug")
                    && std::env::var_os("XSSI_BUILD_METADATA_DIR").is_none(),
                "unbound identity is only valid for a development build"
            );
            return "unbound".to_owned();
        }
        if is_valid_git_sha(value) {
            return value.to_ascii_lowercase();
        }
        panic!("XCZS_SOURCE_REVISION must contain 7 to 64 hexadecimal characters");
    }

    git_output(&["rev-parse", "--short=12", "HEAD"])
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| is_valid_git_sha(value))
        .unwrap_or_else(|| "unknown".to_string())
}

fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let output = String::from_utf8(output.stdout).ok()?;
    let output = output.strip_suffix('\n').unwrap_or(&output);
    let output = output.strip_suffix('\r').unwrap_or(output);
    (!output.is_empty() && !output.contains(['\n', '\r'])).then(|| output.to_owned())
}

fn is_valid_git_sha(value: &str) -> bool {
    (7..=64).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn emit_offline_metadata() {
    use sha2::{Digest, Sha256};
    use xcss::contracts::{StateContract, StateResource, StateResourceKind, StateSchemaIdentity};
    let Some(destination) = std::env::var_os("XSSI_BUILD_METADATA_DIR") else {
        return;
    };
    let destination = std::path::PathBuf::from(destination);
    assert!(
        destination.is_absolute() && destination.is_dir(),
        "explicit metadata destination must exist and be absolute"
    );
    let revision = std::env::var("XCZS_SOURCE_REVISION")
        .expect("release metadata requires a source-bound build");
    let hash = &xcss::sqlite::fingerprint_trusted_ddl(&[&format!(
        "{}\n{}",
        include_str!("schema/product.sql"),
        xcss::admin_sqlite::ADMIN_PERSISTENT_DDL
    )])
    .expect("schema hash");
    let schema =
        xcss::contracts::SchemaIdentity::new("xczs", "1.0.0", 2, hash).expect("schema identity");
    let contract = StateContract {
        contract_version: xcss::contracts::STATE_CONTRACT_VERSION,
        application: "xczs".into(),
        application_version: std::env::var("CARGO_PKG_VERSION").unwrap(),
        source_revision: revision.clone(),
        schema: Some(StateSchemaIdentity {
            revision: 2,
            sha256: hash.into(),
        }),
        maintenance_locks: vec![
            ".state-instance.lock".into(),
            ".state-maintenance.lock".into(),
        ],
        resources: vec![
            StateResource {
                name: "state".into(),
                kind: StateResourceKind::DataTree,
                required: true,
            },
            StateResource {
                name: "config".into(),
                kind: StateResourceKind::Configuration,
                required: true,
            },
        ],
        external_requirements: vec![],
        companion_contracts: vec![],
    };
    contract.validate().expect("current state contract");
    let bytes = serde_json::to_vec(&contract).unwrap();
    let digest = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let identity = xcss::contracts::ReleaseIdentity {
        product: "xczs".into(),
        version: std::env::var("CARGO_PKG_VERSION").unwrap(),
        source_revision: revision,
        target: xcss::server_target::SERVER_TARGET_TRIPLE.into(),
        state_contract_sha256: digest,
    };
    identity.validate().expect("release identity");
    std::fs::write(
        destination.join("xczs-identity.json"),
        serde_json::to_vec(&identity).unwrap(),
    )
    .unwrap();
    std::fs::write(
        destination.join("xczs-schema.json"),
        serde_json::to_vec(&schema).unwrap(),
    )
    .unwrap();
    std::fs::write(destination.join("xczs-state-contract.json"), bytes).unwrap();
    let generated = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap())
        .join("xcss-web-assets.rs.manifest.json");
    std::fs::copy(generated, destination.join("xczs-web-assets.json"))
        .expect("copy exact embedded Web manifest");
}
