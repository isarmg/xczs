//! Product-owned database identity shared by runtime and build metadata.
pub const APPLICATION: &str = "xczs";
pub const APPLICATION_VERSION: &str = "xczs-db-v1";
pub const SCHEMA_REVISION: i64 = 1;

pub fn identity(
    sha256: &str,
) -> Result<xcss::schema_identity::SchemaIdentity, xcss::schema_identity::Error> {
    xcss::schema_identity::SchemaIdentity::new(
        APPLICATION,
        APPLICATION_VERSION,
        SCHEMA_REVISION as u64,
        sha256,
    )
}
