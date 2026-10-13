use crate::http_logger::HttpLogger;
use http::{Method, Request};
use std::collections::HashMap;

/// Data parsed once at the HTTP boundary and shared by request handlers.
///
/// Normalized paths and authentication data can move here incrementally
/// without changing the transport entry point again.
#[derive(Debug)]
pub struct RequestContext {
    head_request: bool,
    access_log: HashMap<String, String>,
}

impl RequestContext {
    pub fn new<B>(request: &Request<B>, logger: &HttpLogger) -> Self {
        let access_log = logger.data(request);
        Self {
            head_request: request.method() == Method::HEAD,
            access_log,
        }
    }

    pub fn is_head_request(&self) -> bool {
        self.head_request
    }

    pub fn access_log(&self) -> &HashMap<String, String> {
        &self.access_log
    }

    pub fn access_log_mut(&mut self) -> &mut HashMap<String, String> {
        &mut self.access_log
    }
}
