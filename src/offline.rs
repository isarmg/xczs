//! Current offline installation and explicit migration adapters for the signed setup helper.
use crate::{Args, server::Server};
use anyhow::{Context, Result};
use std::path::Path;

pub async fn validate_config(root: &Path, config: &Path) -> Result<xcss::offline::ConfigReport> {
    let args = Args::load(root, config)?;
    let data_dir = args.state_dir.clone().context("data_dir is required")?;
    let mut state_paths = vec![root.join("config"), root.join("logs"), data_dir.clone()];
    if !state_paths
        .iter()
        .any(|parent| args.serve_path.starts_with(parent))
    {
        state_paths.push(args.serve_path.clone());
    }
    Ok(xcss::offline::ConfigReport {
        product: "xczs".into(),
        root: root.into(),
        data_dir: data_dir.clone(),
        state_paths,
        shared_paths: vec![args.serve_path.clone()],
        schema_identity: Server::expected_schema_identity()?,
        bind: args.bind,
        writable_paths: vec![
            data_dir,
            args.serve_path,
            root.join("logs"),
            root.join("run"),
        ],
    })
}

pub async fn initialize(root: &Path, config: &Path, username: &str, password: &str) -> Result<()> {
    let args = Args::load(root, config)?;
    Server::initialize(args, username, password)
}

pub async fn check(root: &Path, config: &Path) -> Result<xcss::offline::CheckReport> {
    let report = validate_config(root, config).await?;
    let args = Args::load(root, config)?.validate()?;
    Server::validate_current(&args)?;
    Ok(xcss::offline::CheckReport {
        config: report,
        initialized: true,
    })
}

pub async fn reset_password(
    root: &Path,
    config: &Path,
    username: &str,
    password: &str,
) -> Result<()> {
    let args = Args::load(root, config)?.validate()?;
    let state = xcss::state_file::PrivateStateDirectory::open(
        args.state_dir.as_ref().context("data_dir is required")?,
    )?;
    let _maintenance = state.try_maintenance_lock()?;
    state.verify_no_pending_maintenance()?;
    Server::validate_current(&args)?;
    let pool = Server::administrator_pool(&args).await?;
    let now = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_micros(),
    )?;
    let service = xcss::admin_core::AdministratorService::new(
        xcss::admin_sqlite::SqliteAdministratorStore::new(pool.clone()),
    );
    let result = service
        .change_administrator_password(username, password, now)
        .await;
    pool.close().await;
    result?;
    Ok(())
}

pub async fn migrate(
    root: &Path,
    config: &Path,
    data_dir: &Path,
    source: &xcss::contracts::SchemaIdentity,
    target: &xcss::contracts::SchemaIdentity,
) -> Result<()> {
    let report = validate_config(root, config).await?;
    anyhow::ensure!(
        source == target && target == &report.schema_identity,
        "XCZS does not support the requested schema migration"
    );
    anyhow::ensure!(
        data_dir == report.data_dir,
        "migration data_dir differs from the locked instance"
    );
    check(root, config).await?;
    Ok(())
}
