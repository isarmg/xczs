//! Persistent administrator credentials and sessions share the private state database.
use anyhow::Result;
use std::sync::Arc;
use xcss::admin_core::{AdministratorRecord, AdministratorService, Identifier};
use xcss::admin_sqlite::SqliteAdministratorStore;

/// Stable administrator ID used by durable file operations and their ACLs.
#[derive(Clone, Debug)]
pub struct FilePrincipal {
    pub username: String,
}

pub(crate) async fn initialize_administrator(
    connection: &mut sqlx::SqliteConnection,
    username: &str,
    password: &str,
) -> Result<()> {
    xcss::admin_auth::require_canonical_administrator_username(username)?;
    let now = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_micros(),
    )?;
    let record = AdministratorRecord {
        administrator_id: Identifier::new(uuid::Uuid::new_v4().to_string())?,
        username: username.into(),
        password_hash: xcss::admin_auth::hash_password(password)?,
        active: true,
        session_version: 1,
        created_at_micros: now,
        updated_at_micros: now,
        last_login_at_micros: None,
    };
    insert_administrator(connection, &record).await
}

pub(crate) async fn insert_administrator(
    connection: &mut sqlx::SqliteConnection,
    record: &AdministratorRecord,
) -> Result<()> {
    xcss::admin_sqlite::insert_administrator_on_connection(connection, record).await?;
    Ok(())
}

pub(crate) fn administrator_service(
    pool: sqlx::SqlitePool,
) -> Arc<AdministratorService<SqliteAdministratorStore>> {
    Arc::new(AdministratorService::new(SqliteAdministratorStore::new(
        pool,
    )))
}
