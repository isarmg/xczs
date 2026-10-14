#[macro_use]
extern crate log;

// Depending on the shared gate makes unsupported server targets fail during
// compilation, while this public identity prevents release metadata drift.
pub const SERVER_TARGET: &str = xcss::server_target::SERVER_TARGET_TRIPLE;

pub mod app_error;
pub mod args;
pub mod auth;
mod config_definition;
pub mod http_logger;
pub mod http_utils;
mod log_format_definition;
pub mod logger;
mod persistent_definition;
mod schema_definition;

pub mod request_context;
pub mod server;
pub mod utils;

pub use args::Args;

#[cfg(test)]
#[path = "../tests/support/administrator.rs"]
mod test_support;
