//! Isolated test fixture initialization. This example is never installed or packaged.
use std::{io::Read, path::PathBuf};
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut arguments = std::env::args().skip(1);
    anyhow::ensure!(
        arguments.next().as_deref() == Some("--root"),
        "fixture root required"
    );
    let root = PathBuf::from(
        arguments
            .next()
            .ok_or_else(|| anyhow::anyhow!("fixture root missing"))?,
    );
    anyhow::ensure!(
        arguments.next().as_deref() == Some("--username"),
        "fixture username required"
    );
    let username = arguments
        .next()
        .ok_or_else(|| anyhow::anyhow!("fixture username missing"))?;
    anyhow::ensure!(arguments.next().is_none(), "unexpected fixture argument");
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((xcss::admin_auth::PASSWORD_MAX_BYTES + 3) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.ends_with(b"\n") {
        bytes.pop();
        if bytes.ends_with(b"\r") {
            bytes.pop();
        }
    }
    let password = String::from_utf8(bytes)?;
    xczs::offline::initialize(
        &root,
        &root.join("config/config.json"),
        &username,
        &password,
    )
    .await
}
