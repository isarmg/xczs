//! Current shared-root bindings used by runtime and generated installation data.
#[allow(dead_code)] // Consumed by the build exporter and isolated current-schema fixtures.
pub(crate) const INITIALIZE_SHARED_ROOT: &str =
    include_str!("../schema/initialize-shared-root.sql");
pub(crate) const CHECK_SHARED_ROOT: &str = include_str!("../schema/check-shared-root.sql");
#[allow(dead_code)] // Consumed by the build exporter and isolated current-schema fixtures.
pub(crate) const INITIALIZE_TAG_ROOT: &str = include_str!("../schema/initialize-tag-root.sql");
pub(crate) const CHECK_TAG_ROOT: &str = include_str!("../schema/check-tag-root.sql");
