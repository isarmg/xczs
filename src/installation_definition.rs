//! Export product-owned current definitions; independent tools execute them.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;
use xcss::contracts::{ReleaseIdentity, SchemaIdentity};

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn root(value: &str) -> Value {
    json!({"base":"root","value":value,"fallback":null,"suffix":"","encoding":"path"})
}
fn data(value: &str) -> Value {
    let mut path = crate::config_definition::data_path_spec();
    path["suffix"] = json!(value);
    path
}
fn asset(directory: &Path, name: &str, bytes: &[u8]) -> Value {
    std::fs::write(directory.join(name), bytes).unwrap();
    json!({"path":format!("share/contracts/{name}"),"sha256":sha(bytes)})
}
fn resource(source: &str, kind: &str) -> Value {
    json!({"source":source,"value":"/resources/shared_paths/0","type":kind})
}
fn relocate(value: &mut Value) {
    match value {
        Value::String(text) => {
            if let Some(suffix) = text.strip_prefix("/opt/isarmg/xczs/") {
                *text = format!("$ROOT/{suffix}");
            }
        }
        Value::Array(items) => items.iter_mut().for_each(relocate),
        Value::Object(items) => items.values_mut().for_each(relocate),
        _ => {}
    }
}
pub(crate) fn emit(
    destination: &Path,
    identity: &ReleaseIdentity,
    schema: &SchemaIdentity,
    ddl: &str,
) {
    let directory = destination.join("contracts");
    std::fs::create_dir_all(&directory).unwrap();
    let mut material = xcss::installation_material::common(identity, schema, None).unwrap();
    let definitions = material
        .as_object_mut()
        .unwrap()
        .remove("definitions")
        .unwrap();
    asset(
        &directory,
        "common-definitions.json",
        &serde_json::to_vec(&definitions).unwrap(),
    );
    let mut database = material
        .as_object_mut()
        .unwrap()
        .remove("database")
        .unwrap();
    database["name"] = json!("state");
    database["path"] = data("state.sqlite3");
    database["ddl"] = asset(&directory, "current-schema.sql", ddl.as_bytes());
    database["initialize"].as_array_mut().unwrap().push(json!({"sql":crate::persistent_definition::INITIALIZE_SHARED_ROOT,
        "parameters":[resource("resource_device_be","blob"),resource("resource_inode_be","blob")],"expected_changes":2}));
    database["checks"].as_array_mut().unwrap().push(json!({"name":"current-shared-root-identity","sql":crate::persistent_definition::CHECK_SHARED_ROOT,
        "parameters":[resource("resource_device_be","blob"),resource("resource_inode_be","blob")],"expect":"true"}));
    let tag_ddl = include_str!("../schema/tagging.sql");
    let tag_hash = xcss::sqlite::fingerprint_trusted_ddl(&[tag_ddl]).unwrap();
    let tag_identity = SchemaIdentity::new("xczs-tags", "1.0.0", 1, &tag_hash).unwrap();
    let tags = json!({"name":"tags","path":data("tags.db"),"ddl":asset(&directory,"tag-schema.sql",tag_ddl.as_bytes()),
        "schema_identity":tag_identity,"schema_stamp":null,"fingerprint_algorithm":"sqlite-schema-v1",
        "initialize":[{"sql":crate::persistent_definition::INITIALIZE_TAG_ROOT,"parameters":[resource("resource_path","text")],"expected_changes":1}],
        "reset_administrator":[],"checks":[{"name":"current-tag-root","sql":crate::persistent_definition::CHECK_TAG_ROOT,
            "parameters":[resource("resource_path","text")],"expect":"true"}],"validation":[]});
    material["databases"] = json!([database, tags]);
    let mut config = xcss::configuration_contract::generate::<
        crate::config_definition::ArgsDefinition<String, ()>,
    >()
    .unwrap();
    config["schema"].as_object_mut().unwrap().remove("required");
    relocate(&mut config["defaults"]);
    config["data_dir"] = crate::config_definition::data_path_spec();
    config["bind_pointer"] = json!("/bind");
    config["rules"] = crate::config_definition::current_rules(true, true);
    let mut baseline = config["defaults"].clone();
    baseline["serve_path"] = json!("$ROOT/files");
    let mut cases = vec![
        json!({"name":"explicit-shared-root-with-current-defaults","value":baseline,"valid":true}),
        json!({"name":"shared-root-must-be-selected","value":{},"valid":false}),
    ];
    for (name, field, value, valid) in [
        ("unknown-property", "retired_field", json!(true), false),
        (
            "relative-shared-root",
            "serve_path",
            json!("relative"),
            false,
        ),
        (
            "shared-private-overlap",
            "serve_path",
            json!("$ROOT/state/files"),
            false,
        ),
        (
            "shared-contains-private",
            "serve_path",
            json!("$ROOT"),
            false,
        ),
        (
            "protected-installation-namespace",
            "data_dir",
            json!("$ROOT/config/state"),
            false,
        ),
        ("private-data-required", "data_dir", Value::Null, false),
        ("unknown-log-level", "log_level", json!("invalid"), false),
        ("zero-concurrency", "max_connections", json!(0), false),
        ("invalid-bind", "bind", json!("not-a-socket"), false),
        (
            "search-over-bound",
            "max_search_entries",
            json!(100001),
            false,
        ),
        ("zero-timeout", "upload_idle_timeout", json!(0), false),
        (
            "total-smaller-than-idle",
            "upload_total_timeout",
            json!(1),
            false,
        ),
        (
            "unknown-log-variable",
            "log_format",
            json!("$removed$status"),
            false,
        ),
        (
            "adjacent-log-variables",
            "log_format",
            json!("$status$request"),
            true,
        ),
        ("empty-log-format", "log_format", json!(""), true),
        ("empty-request-header", "log_format", json!("$http_"), false),
        (
            "unicode-request-header",
            "log_format",
            json!("$http_文"),
            false,
        ),
        (
            "utf8-log-byte-bound",
            "log_format",
            json!("文".repeat(1365)),
            true,
        ),
        (
            "utf8-log-byte-overflow",
            "log_format",
            json!("文".repeat(1366)),
            false,
        ),
        (
            "log-token-bound",
            "log_format",
            json!("$status".repeat(128)),
            true,
        ),
        (
            "log-token-overflow",
            "log_format",
            json!("$status".repeat(129)),
            false,
        ),
    ] {
        if field == "log_format" {
            let tokens = crate::log_format_definition::parse(value.as_str().unwrap());
            assert_eq!(
                tokens.is_ok(),
                valid,
                "owned log-format vector must reflect the runtime parser"
            );
            if let Ok(tokens) = tokens {
                for token in tokens {
                    if let crate::log_format_definition::Token::Variable(variable) = token {
                        assert!(crate::log_format_definition::VARIABLES.contains(&variable.name()));
                    }
                }
            }
        }
        let mut value_case = baseline.clone();
        value_case[field] = value;
        cases.push(json!({"name":name,"value":value_case,"valid":valid}));
    }
    config["cases"] = json!(cases);
    material["config"] = config;
    let state = crate::config_definition::data_path_spec();
    let shared = crate::config_definition::shared_path_spec();
    material["resources"] = json!({"state_paths":[state,root("config")],
        "writable_paths":[state,shared,root("logs"),root("run")],"readonly_paths":[root("config/config.json")],
        "shared_paths":[shared],"business_paths":[shared],"business_directories":[],"config_write_paths":[],
        "private_directories":[state,root("logs"),root("run")],
        "runtime_locks":[{"path":data(".state-instance.lock"),"kind":"file"},{"path":shared,"kind":"directory"}]});
    material["secrets"] = json!([]);
    material["media"] = Value::Null;
    material["writers"] = json!([{"role":"server","service":"xczs.service","configuration_paths":[root("config/config.json")]}]);
    for (path, bytes) in [
        (
            "src/config_definition.rs",
            include_bytes!("config_definition.rs").as_slice(),
        ),
        (
            "src/log_format_definition.rs",
            include_bytes!("log_format_definition.rs").as_slice(),
        ),
        (
            "src/persistent_definition.rs",
            include_bytes!("persistent_definition.rs").as_slice(),
        ),
        (
            "src/installation_definition.rs",
            include_bytes!("installation_definition.rs").as_slice(),
        ),
        (
            "schema/product.sql",
            include_bytes!("../schema/product.sql").as_slice(),
        ),
        (
            "schema/tagging.sql",
            include_bytes!("../schema/tagging.sql").as_slice(),
        ),
        (
            "schema/initialize-shared-root.sql",
            include_bytes!("../schema/initialize-shared-root.sql").as_slice(),
        ),
        (
            "schema/check-shared-root.sql",
            include_bytes!("../schema/check-shared-root.sql").as_slice(),
        ),
        (
            "schema/initialize-tag-root.sql",
            include_bytes!("../schema/initialize-tag-root.sql").as_slice(),
        ),
        (
            "schema/check-tag-root.sql",
            include_bytes!("../schema/check-tag-root.sql").as_slice(),
        ),
    ] {
        material["authority_sources"].as_array_mut().unwrap().push(json!({"path":path,"sha256":sha(bytes),"generator":format!("xczs/build.rs@{}",identity.source_revision)}));
    }
    asset(
        &directory,
        "install.json",
        &serde_json::to_vec(&material).unwrap(),
    );
}
