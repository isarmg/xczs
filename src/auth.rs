//! Administrator credentials live only in the protected xcss account store.
use anyhow::{Result, ensure};
use std::sync::Arc;
use xcss::admin_core::{AdministratorRecord, AdministratorService, Identifier};
use xcss::admin_static::StaticAdministratorStore;

/// Stable administrator ID used by durable file operations and their ACLs.
#[derive(Clone, Debug)]
pub struct FilePrincipal {
    pub username: String,
}

pub(crate) fn initialize_administrator(
    directory: xcss::fs_safety::PrivateDirectory,
    username: &str,
    password: &str,
) -> Result<()> {
    xcss::admin_auth::require_canonical_administrator_username(username)?;
    let record = AdministratorRecord {
        administrator_id: Identifier::new(username)?,
        username: username.to_owned(),
        password_hash: xcss::admin_auth::hash_password(password)?,
        active: true,
        session_version: 1,
        created_at_micros: 0,
        updated_at_micros: 0,
        last_login_at_micros: None,
    };
    StaticAdministratorStore::with_persistent_accounts([record], directory)?;
    Ok(())
}

pub(crate) fn administrator_service_with_directory(
    directory: xcss::fs_safety::PrivateDirectory,
) -> Result<Arc<AdministratorService<StaticAdministratorStore>>> {
    let records = StaticAdministratorStore::load_persistent_accounts(&directory, None)?;
    ensure!(
        records.len() == 1,
        "Exactly one initialized administrator is required"
    );
    Ok(Arc::new(AdministratorService::new(
        StaticAdministratorStore::with_persistent_accounts(records, directory)?,
    )))
}

#[cfg(test)]
mod tests;
