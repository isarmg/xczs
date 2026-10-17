#[path = "../xcss_web_assets.rs"]
mod xcss_web_assets;
use super::{Response, Server, status_not_found};
use http::{
    HeaderMap, Method,
    header::{CACHE_CONTROL, HeaderValue},
};
include!(concat!(env!("OUT_DIR"), "/xcss-web-assets.rs"));

pub fn web_assets_manifest() -> anyhow::Result<&'static str> {
    xcss::web_assets::verify_embedded(ASSETS, MANIFEST, DIGEST)?;
    Ok(MANIFEST)
}
pub(super) fn wasm_preload(assets_prefix: &str) -> String {
    ASSETS
        .iter()
        .find(|asset| asset.path.ends_with(".wasm"))
        .map(|asset| {
            format!(
                "<link rel=\"preload\" href=\"/{assets_prefix}{}\" as=\"fetch\" type=\"application/wasm\" crossorigin=\"anonymous\">",
                asset.path
            )
        })
        .unwrap_or_default()
}

pub(super) fn embedded_assets_prefix() -> String {
    format!("__xczs_assets_{DIGEST}/")
}
impl Server {
    pub(super) fn handle_internal(
        &self,
        req_path: &str,
        method: &Method,
        headers: &HeaderMap,
        res: &mut Response,
    ) -> bool {
        let Some(name) = req_path.strip_prefix(&self.content.assets_prefix) else {
            return false;
        };
        if let Some(directory) = &self.content.development_web {
            *res = directory
                .response(name, method, headers)
                .map(axum::body::Body::from);
            return true;
        }
        if !ASSETS.iter().any(|asset| asset.path == name) {
            status_not_found(res);
            return true;
        }
        *res = xcss_web_assets::response(name, method, headers, |path, method, headers| {
            xcss::web_assets::response(ASSETS, path, method, headers)
        })
        .map(axum::body::Body::from);
        // The complete inventory digest is part of this URL, including fixed filenames.
        res.headers_mut().insert(
            CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
        true
    }
    pub(super) fn is_public_asset_path(&self, req_path: &str) -> bool {
        req_path
            .strip_prefix(&self.content.assets_prefix)
            .is_some_and(|name| {
                self.content.development_web.is_some()
                    || ASSETS.iter().any(|asset| asset.path == name)
            })
    }
}
