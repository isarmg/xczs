use crate::protocol;
use js_sys::{Reflect, Uint8Array};
use serde_json::{Value, json};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AbortController, Headers, ReadableStreamDefaultReader, RequestCredentials, RequestInit,
    Response,
};
use xcss_web::i18n::{locale, t};

pub const CSRF: &str = "X-CSRF-Token";
pub const OP_ID: &str = "X-Xczs-Operation-Id";
pub const OP_STATE: &str = "X-Xczs-Operation-State";
pub const UP_ID: &str = "X-Xczs-Upload-Id";
pub const UP_LENGTH: &str = "X-Xczs-Upload-Length";
pub const UP_OFFSET: &str = "X-Xczs-Upload-Offset";
pub const UP_OVERWRITE: &str = "X-Xczs-Upload-Overwrite";
pub const TARGET: &str = "X-Xczs-Target-Revision";
pub const ERROR_LIMIT: usize = 16 * 1024;
pub const SUCCESS_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct Error {
    pub status: u16,
    pub problem_status: u16,
    pub code: String,
    pub message: String,
    pub recovery: String,
    pub retry_after: Option<u64>,
    pub retryable: bool,
    pub unknown: bool,
    pub operation_id: String,
    pub operation_state: String,
    pub target_revision: Option<String>,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Error {}
impl Error {
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ..Self::default()
        }
    }
    pub fn network(unknown: bool) -> Self {
        Self {
            message: if unknown {
                unknown_message()
            } else {
                t(
                    "网络连接中断，请重试。",
                    "Network connection lost. Try again.",
                )
            },
            unknown,
            ..Self::default()
        }
    }
    pub fn trusted(&self, status: u16, code: &str) -> bool {
        self.status == status && self.problem_status == status && self.code == code && !self.unknown
    }
    pub fn auth(&self) -> bool {
        self.status == 401 || (self.status == 403 && self.code == "auth.csrf_rejected")
    }
}
pub fn unknown_message() -> String {
    t(
        "结果不确定，请刷新文件夹核对实际状态后再重试。",
        "The result is unknown; refresh the folder to verify what happened before trying again.",
    )
}
pub fn window() -> web_sys::Window {
    web_sys::window().expect("browser window")
}
pub fn document() -> web_sys::Document {
    window().document().expect("browser document")
}
pub fn uuid() -> String {
    window()
        .crypto()
        .expect("secure browser crypto")
        .random_uuid()
}
pub fn token() -> String {
    xcss_web::api::current_session()
        .map(|s| s.csrf_token)
        .unwrap_or_default()
}
pub fn encode(value: &str) -> String {
    js_sys::encode_uri_component(value)
        .as_string()
        .unwrap_or_default()
}
pub fn file_url(path: &str) -> String {
    path.split('/').map(encode).collect::<Vec<_>>().join("/")
}
pub fn unauthorized() {
    let _ = window().location().reload();
}
pub fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\0')
        && path.len() <= 4095
        && path
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != ".." && s.len() <= 255)
}
pub fn query(values: impl IntoIterator<Item = (impl AsRef<str>, impl AsRef<str>)>) -> String {
    values
        .into_iter()
        .filter_map(|(k, v)| {
            let v = v.as_ref();
            (!v.is_empty()).then(|| format!("{}={}", encode(k.as_ref()), encode(v)))
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn platform_error_code<'a>(value: &'a Value, content_type: &str) -> Option<&'a str> {
    if !content_type
        .split(';')
        .next()?
        .trim()
        .eq_ignore_ascii_case("application/json")
    {
        return None;
    }
    let object = value.as_object()?;
    if !object.keys().all(|key| {
        ["code", "message", "retryable", "request_id", "details"].contains(&key.as_str())
    }) {
        return None;
    }
    let code = value["code"].as_str()?;
    if !(1..=128).contains(&code.len())
        || !code.as_bytes()[0].is_ascii_lowercase()
        || !code
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        || !value["message"].is_string()
        || !value["retryable"].is_boolean()
        || value
            .get("request_id")
            .is_some_and(|v| !v.as_str().is_some_and(xcss_web::contracts::safe_request_id))
        || value.get("details").is_some_and(|v| !v.is_object())
    {
        return None;
    }
    Some(code)
}
fn business_code(value: &Value) -> String {
    value
        .as_str()
        .filter(|code| {
            (1..=64).contains(&code.len())
                && code.as_bytes()[0].is_ascii_lowercase()
                && code
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
        .unwrap_or_default()
        .to_owned()
}
fn recovery_advice(value: &Value) -> String {
    value
        .as_str()
        .filter(|value| {
            matches!(
                *value,
                "retry"
                    | "retry_with_new_id"
                    | "resume_upload"
                    | "query_job"
                    | "query_upload"
                    | "refresh_target"
            )
        })
        .unwrap_or_default()
        .to_owned()
}
fn body_retry_after(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .filter(|n| *n <= crate::model::MAX_SAFE_INTEGER)
}
fn canonical_uuid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}
#[derive(Clone)]
pub struct Raw {
    pub status: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
}
impl Raw {
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers.get(name).ok().flatten()
    }
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
    pub fn json(&self) -> Result<Value, Error> {
        let media = self
            .header("content-type")
            .unwrap_or_default()
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        if media != "application/json"
            && !(media.starts_with("application/") && media.ends_with("+json"))
        {
            return Err(Error::message(t(
                "服务器返回了无效 JSON 类型",
                "The server returned an invalid JSON content type",
            )));
        }
        serde_json::from_slice(&self.body).map_err(|_| {
            Error::message(t(
                "服务器返回了无效 JSON",
                "The server returned invalid JSON",
            ))
        })
    }
    pub fn error(&self, unknown: bool) -> Error {
        self.error_with_reload(unknown, true)
    }
    pub fn upload_error(&self, unknown: bool) -> Error {
        let mut error = self.error_with_reload(unknown, false);
        if error.status == 401 {
            error.message = t(
                "会话已失效，正在返回登录页面。",
                "Your session is no longer valid. Returning to the sign-in page.",
            );
        } else if error.status == 403 && error.code == "auth.csrf_rejected" {
            error.message = t(
                "会话或页面已失效。请刷新页面后重新选择文件。",
                "Your session or this page is no longer valid. Refresh the page and select the files again.",
            );
        }
        error
    }
    fn error_with_reload(&self, unknown: bool, reload_auth: bool) -> Error {
        let problem = self.header("content-type").is_some_and(|v| {
            v.split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .eq_ignore_ascii_case("application/problem+json")
        });
        let json_media = self.header("content-type").is_some_and(|v| {
            let media = v
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            media == "application/json" || media.ends_with("+json")
        });
        let body = if json_media {
            serde_json::from_slice::<Value>(&self.body).unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        let text = |key: &str| {
            body[key]
                .as_str()
                .filter(|s| s.len() <= 2048)
                .unwrap_or_default()
                .to_owned()
        };
        let problem_status = if problem {
            body["status"]
                .as_u64()
                .filter(|s| (100..600).contains(s))
                .unwrap_or(0) as u16
        } else {
            0
        };
        let code = if problem {
            let code = business_code(&body["code"]);
            if code == "auth.csrf_rejected" {
                String::new()
            } else {
                code
            }
        } else {
            platform_error_code(&body, &self.header("content-type").unwrap_or_default())
                .unwrap_or_default()
                .to_owned()
        };
        let auth = self.status == 401 || (self.status == 403 && code == "auth.csrf_rejected");
        let contradictory = problem_status != 0 && problem_status != self.status;
        let detail = if problem {
            text("detail")
        } else {
            String::new()
        };
        let message = if auth {
            t(
                "会话或页面已失效，请刷新页面。",
                "Your session or page has expired. Refresh the page.",
            )
        } else if contradictory {
            t(
                "错误响应无效：问题状态与 HTTP 状态不一致",
                "Invalid error response: problem status does not match HTTP status",
            )
        } else if locale() == "en" && !detail.is_empty() {
            detail
        } else {
            match code.as_str() {
                "path_exists" | "destination_exists" => t("路径已存在", "Path already exists"),
                "source_changed" | "destination_changed" | "delete_target_changed" => t(
                    "目标状态已变化，请刷新后重试。",
                    "Target changed. Refresh and review.",
                ),
                "permission_denied" => t("权限不足", "Permission denied"),
                "insufficient_storage" => t("存储空间不足", "Insufficient storage"),
                _ => format!(
                    "{} (HTTP {})",
                    t(
                        "请求失败，请核对目标状态后再重试。",
                        "Request failed; check the target state before retrying."
                    ),
                    self.status
                ),
            }
        };
        let retry_after = match self.header("retry-after") {
            Some(v) => protocol::unsigned(&v),
            None => body_retry_after(&body["retry_after"]),
        };
        let e = Error {
            status: self.status,
            problem_status,
            code: if contradictory {
                "invalid_problem_status".into()
            } else {
                code
            },
            message,
            recovery: if contradictory || !problem {
                String::new()
            } else {
                recovery_advice(&body["recovery"])
            },
            retry_after,
            retryable: body["retryable"].as_bool().unwrap_or(false),
            unknown: unknown && !auth,
            operation_id: self.header(OP_ID).unwrap_or_default(),
            operation_state: self.header(OP_STATE).unwrap_or_default(),
            target_revision: self.header(TARGET).filter(|v| protocol::revision(v)),
        };
        if auth && reload_auth {
            gloo_timers::callback::Timeout::new(0, unauthorized).forget();
        }
        e
    }
}

pub async fn raw(
    path: &str,
    method: &str,
    headers: Vec<(String, String)>,
    body: Option<JsValue>,
    controller: Option<AbortController>,
    timeout: u32,
    unknown: bool,
) -> Result<Raw, Error> {
    raw_bounded(
        path,
        method,
        headers,
        body,
        controller,
        ResponseBounds {
            timeout,
            unknown,
            success_limit: SUCCESS_LIMIT,
        },
    )
    .await
}
pub struct ResponseBounds {
    pub timeout: u32,
    pub unknown: bool,
    pub success_limit: usize,
}
struct AbortLink {
    signal: web_sys::AbortSignal,
    listener: Closure<dyn Fn(web_sys::Event)>,
}
impl Drop for AbortLink {
    fn drop(&mut self) {
        let _ = self
            .signal
            .remove_event_listener_with_callback("abort", self.listener.as_ref().unchecked_ref());
    }
}
fn request_controller(
    source: Option<AbortController>,
) -> Result<(AbortController, Option<AbortLink>), Error> {
    let controller = AbortController::new().map_err(|_| Error::network(false))?;
    let link = if let Some(source) = source {
        let signal = source.signal();
        if signal.aborted() {
            return Err(Error::message(t("请求已取消。", "Request cancelled.")));
        }
        let target = controller.clone();
        let listener = Closure::<dyn Fn(web_sys::Event)>::new(move |_| target.abort());
        signal
            .add_event_listener_with_callback("abort", listener.as_ref().unchecked_ref())
            .map_err(|_| Error::network(false))?;
        Some(AbortLink { signal, listener })
    } else {
        None
    };
    Ok((controller, link))
}
pub async fn raw_bounded(
    path: &str,
    method: &str,
    headers: Vec<(String, String)>,
    body: Option<JsValue>,
    controller: Option<AbortController>,
    bounds: ResponseBounds,
) -> Result<Raw, Error> {
    let ResponseBounds {
        timeout,
        unknown,
        success_limit,
    } = bounds;
    // The caller's signal denotes a superseded view or explicit cancellation.
    // Deadlines abort only this linked controller, so callers still surface a
    // timeout instead of silently discarding it as an obsolete view response.
    let (controller, _abort_link) = request_controller(controller)?;
    let init = RequestInit::new();
    init.set_method(method);
    init.set_credentials(RequestCredentials::SameOrigin);
    init.set_signal(Some(&controller.signal()));
    let h = Headers::new().map_err(|_| Error::network(false))?;
    for (k, v) in headers {
        h.set(&k, &v)
            .map_err(|_| Error::message("Invalid request header"))?;
    }
    init.set_headers(&h);
    if let Some(body) = body.as_ref() {
        init.set_body(body);
    }
    let timed = controller.clone();
    let timed_out = std::rc::Rc::new(std::cell::Cell::new(false));
    let expired = timed_out.clone();
    let timer = gloo_timers::callback::Timeout::new(timeout, move || {
        expired.set(true);
        timed.abort();
    });
    let result = async {
        let response: Response = JsFuture::from(window().fetch_with_str_and_init(path, &init))
            .await
            .map_err(|_| Error::network(unknown))?
            .dyn_into()
            .map_err(|_| Error::network(unknown))?;
        let status = response.status();
        let headers = response.headers();
        if status == 401 {
            // Authentication is authoritative at headers. A stalled or huge
            // body must not delay navigation or trigger job reconciliation.
            controller.abort();
            return Ok(Raw {
                status,
                headers,
                body: vec![],
            });
        }
        let limit = if response.ok() {
            success_limit
        } else {
            ERROR_LIMIT
        };
        if headers
            .get("content-length")
            .ok()
            .flatten()
            .and_then(|v| protocol::unsigned(&v))
            .is_some_and(|v| v > limit as u64)
        {
            controller.abort();
            return Err(Error {
                status,
                message: t(
                    "响应内容超过限制。",
                    "Response body exceeded the size limit.",
                ),
                unknown,
                ..Error::default()
            });
        }
        let mut bytes = Vec::new();
        if method != "HEAD"
            && let Some(stream) = response.body()
        {
            let reader: ReadableStreamDefaultReader = stream
                .get_reader()
                .dyn_into()
                .map_err(|_| Error::network(unknown))?;
            loop {
                let chunk = JsFuture::from(reader.read())
                    .await
                    .map_err(|_| Error::network(unknown))?;
                if Reflect::get(&chunk, &"done".into())
                    .ok()
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    break;
                }
                let data = Uint8Array::new(
                    &Reflect::get(&chunk, &"value".into()).map_err(|_| Error::network(unknown))?,
                );
                if bytes.len() + data.length() as usize > limit {
                    let _ = reader.cancel();
                    controller.abort();
                    return Err(Error {
                        status,
                        message: t(
                            "响应内容超过限制。",
                            "Response body exceeded the size limit.",
                        ),
                        unknown,
                        ..Error::default()
                    });
                }
                bytes.extend(data.to_vec());
            }
        }
        Ok(Raw {
            status,
            headers,
            body: bytes,
        })
    }
    .await;
    drop(timer);
    if timed_out.get() {
        return Err(Error {
            message: if unknown {
                format!(
                    "{} {}",
                    t("请求超时。", "Request timed out."),
                    unknown_message()
                )
            } else {
                t("请求超时，请重试。", "Request timed out. Try again.")
            },
            code: "client_timeout".into(),
            unknown,
            ..Error::default()
        });
    }
    result
}
pub async fn get(path: &str, controller: Option<AbortController>) -> Result<Value, Error> {
    let r = raw(path, "GET", vec![], None, controller, 30_000, false).await?;
    if !r.ok() {
        return Err(r.error(false));
    }
    r.json()
}
pub async fn json_request(
    path: &str,
    method: &str,
    body: Option<Value>,
    headers: Vec<(String, String)>,
    unknown: bool,
) -> Result<Raw, Error> {
    let mut headers = headers;
    headers.push((CSRF.into(), token()));
    headers.push(("Content-Type".into(), "application/json".into()));
    raw(
        path,
        method,
        headers,
        body.map(|v| JsValue::from_str(&v.to_string())),
        None,
        30_000,
        unknown,
    )
    .await
}
pub async fn mutation(
    path: &str,
    method: &str,
    body: Option<Value>,
    headers: Vec<(String, String)>,
) -> Result<Raw, Error> {
    let id = uuid();
    let mut headers = headers;
    headers.push((OP_ID.into(), id.clone()));
    let result = json_request(path, method, body, headers, true).await;
    let mut error = match result {
        Ok(r) => {
            let state = r.header(OP_STATE).unwrap_or_default();
            let bound = r.header(OP_ID).as_deref() == Some(&id)
                && ["running", "succeeded", "failed", "rejected", "unknown"]
                    .contains(&state.as_str())
                && (!r.ok() || state == "succeeded")
                && (r.ok() || state != "succeeded");
            if matches!(r.status, 401 | 403) {
                let e = r.error(false);
                if e.auth() {
                    return Err(e);
                }
            }
            if bound && r.ok() && r.body.is_empty() {
                return Ok(r);
            }
            if bound && !r.ok() {
                let e = r.error(matches!(state.as_str(), "running" | "unknown"));
                if !e.unknown {
                    return Err(e);
                }
                e
            } else {
                Error {
                    unknown: true,
                    operation_id: id.clone(),
                    message: unknown_message(),
                    ..Error::default()
                }
            }
        }
        Err(mut e) => {
            e.operation_id = id.clone();
            e
        }
    };
    // Mutations are never replayed. One read-only lookup reconciles ambiguous
    // transport failures against the server's operation registry.
    if error.unknown {
        let result = query_job(&id).await;
        if result["kind"] == "succeeded" {
            return Ok(Raw {
                status: 204,
                headers: Headers::new().unwrap(),
                body: vec![],
            });
        }
        if result["authenticationFailed"] == true {
            return Err(Error {
                status: 401,
                message: result["message"].as_str().unwrap_or_default().into(),
                ..Error::default()
            });
        }
        settle_failed_job(&mut error, &result);
        if let Some(message) = result["message"].as_str().filter(|m| !m.is_empty()) {
            error.message = message.to_owned();
        }
    }
    Err(error)
}
fn settle_failed_job(error: &mut Error, result: &Value) {
    if result["kind"] == "failed" {
        error.unknown = false;
        error.status = result["status"].as_u64().unwrap_or_default() as u16;
        error.problem_status = error.status;
        error.code = result["code"].as_str().unwrap_or_default().into();
        error.operation_state = "failed".into();
        // A registry result proves non-commit but cannot recreate conditional
        // overwrite authority from the original response headers.
        error.target_revision = None;
    }
}
/// Validate the same job-status binding used by one-shot mutation reconciliation.
pub fn parse_job(
    payload: &Value,
    id: &str,
    header_id: Option<&str>,
    header_state: Option<&str>,
) -> Option<Value> {
    let state = payload["state"].as_str()?;
    let status = payload["http_status"].as_u64().unwrap_or(0);
    let present = payload.get("http_status").is_some();
    if payload["job_id"].as_str() != Some(id)
        || header_id != Some(id)
        || header_state != Some(state)
        || !matches!(state, "running" | "succeeded" | "failed" | "unknown")
        || state == "running" && present
        || state == "succeeded" && !(200..300).contains(&status)
        || state == "failed" && !(400..600).contains(&status)
        || state == "unknown" && present && !(400..600).contains(&status)
    {
        return None;
    }
    let message = match state {
        "succeeded" => t(
            "服务器已确认操作成功。",
            "The server confirms that the operation succeeded.",
        ),
        "failed" => {
            if locale() == "en" && payload["detail"].as_str().is_some_and(|s| !s.is_empty()) {
                format!(
                    "The server confirms that the operation failed: {}",
                    payload["detail"].as_str().unwrap()
                )
            } else {
                t(
                    "服务器已确认操作失败。",
                    "The server confirms that the operation failed.",
                )
            }
        }
        "unknown" => t(
            "服务器无法证明最终操作结果。请刷新文件夹核对目标后再重试。",
            "The server could not prove the final operation outcome. Refresh the folder to inspect the target before trying again.",
        ),
        _ => t(
            "服务器报告操作仍在进行。请刷新文件夹后再重试。",
            "The server reports that the operation is still running. Refresh the folder before trying again.",
        ),
    };
    Some(
        json!({"kind":state,"jobId":id,"state":state,"status":status,"code":business_code(&payload["code"]),"message":message,"authenticationFailed":false}),
    )
}
pub async fn query_job(id: &str) -> Value {
    if !canonical_uuid(id) {
        return json!({"kind":"unavailable","jobId":id,"state":"unknown","status":0,"code":"invalid_job_id","message":t("任务标识必须为规范的 UUID","Job ID must be a canonical UUID"),"authenticationFailed":false});
    }
    let result = raw(
        &format!("/__xczs__/api/jobs/{id}"),
        "GET",
        vec![],
        None,
        None,
        30_000,
        false,
    )
    .await;
    let error = match result {
        Ok(r) if r.ok() => {
            if let Ok(p) = r.json()
                && let Some(result) = parse_job(
                    &p,
                    id,
                    r.header(OP_ID).as_deref(),
                    r.header(OP_STATE).as_deref(),
                )
            {
                return result;
            }
            Error::message(t("任务状态响应无效", "Invalid job status response"))
        }
        Ok(r) => r.error(false),
        Err(e) => e,
    };
    if error.auth() {
        json!({"kind":"authentication","jobId":id,"state":"unknown","status":0,"code":"auth.session_required","message":error.message,"authenticationFailed":true})
    } else {
        json!({"kind":"unavailable","jobId":id,"state":"unknown","status":0,"code":"job_status_unavailable","message":format!("{} {} {}",unknown_message(),t("本次状态核对失败：","The one-time status check failed:"),error.message),"authenticationFailed":false})
    }
}
pub async fn preflight(paths: &[String]) -> Result<Vec<Target>, Error> {
    preflight_inner(paths, false).await
}
pub async fn upload_preflight(paths: &[String]) -> Result<Vec<Target>, Error> {
    preflight_inner(paths, true).await
}
async fn preflight_inner(paths: &[String], upload: bool) -> Result<Vec<Target>, Error> {
    let r = json_request(
        "/__xczs__/api/upload/preflight",
        "POST",
        Some(json!({"paths":paths})),
        vec![],
        false,
    )
    .await?;
    if !r.ok() {
        return Err(if upload {
            r.upload_error(false)
        } else {
            r.error(false)
        });
    }
    parse_preflight(r.json()?, paths)
}
pub fn parse_preflight(p: Value, paths: &[String]) -> Result<Vec<Target>, Error> {
    let targets = p["targets"]
        .as_array()
        .filter(|a| a.len() == paths.len())
        .ok_or_else(|| {
            Error::message(t("上传预检查响应无效", "Invalid upload preflight response"))
        })?;
    let mut result = Vec::new();
    let mut unique = std::collections::HashSet::new();
    for (p, expected) in targets.iter().zip(paths) {
        let exists = p["exists"].as_bool();
        let replaceable = p["replaceable"].as_bool();
        let rev = p["revision"].as_str().map(str::to_owned);
        if !expected.starts_with('/')
            || !valid_path(&expected[1..])
            || !unique.insert(expected)
            || p["path"].as_str() != Some(expected)
            || exists.is_none()
            || replaceable.is_none()
            || p.get("revision").is_none()
            || if exists == Some(true) {
                rev.as_ref().is_none_or(|v| !protocol::revision(v))
            } else {
                !p["revision"].is_null()
            }
        {
            return Err(Error::message(t(
                "上传预检查响应无效",
                "Invalid upload preflight response",
            )));
        }
        result.push(Target {
            path: expected.clone(),
            exists: exists.unwrap(),
            replaceable: replaceable.unwrap(),
            revision: rev,
        });
    }
    Ok(result)
}
#[derive(Clone, Debug)]
pub struct Target {
    pub path: String,
    pub exists: bool,
    pub replaceable: bool,
    pub revision: Option<String>,
}
pub async fn put_empty(path: &str, id: &str) -> Result<Raw, Error> {
    raw(
        &file_url(path),
        "PUT",
        vec![
            (CSRF.into(), token()),
            (UP_ID.into(), id.into()),
            (UP_LENGTH.into(), "0".into()),
            (UP_OVERWRITE.into(), "false".into()),
        ],
        Some(JsValue::from_str("")),
        None,
        30_000,
        true,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preflight_never_grants_overwrite_for_another_destination() {
        let paths = vec!["/a".to_owned(), "/b".to_owned()];
        let a = json!({"path":"/a","exists":true,"replaceable":true,"revision":"a".repeat(64)});
        let b = json!({"path":"/b","exists":false,"replaceable":true,"revision":null});
        assert!(parse_preflight(json!({"targets":[a.clone(),b.clone()]}), &paths).is_ok());
        assert!(parse_preflight(json!({"targets":[b.clone(),a.clone()]}), &paths).is_err());
        assert!(parse_preflight(json!({"targets":[a.clone()]}), &paths).is_err());
        assert!(
            parse_preflight(
                json!({"targets":[a.clone(),a.clone()]}),
                &["/a".into(), "/a".into()]
            )
            .is_err()
        );
        let mut missing_revision = a;
        missing_revision["revision"] = Value::Null;
        assert!(parse_preflight(json!({"targets":[missing_revision,b]}), &paths).is_err());
    }
    #[test]
    fn paths_enforce_utf8_component_and_traversal_boundaries() {
        assert!(valid_path("a/文件.txt"));
        assert!(!valid_path("/a"));
        assert!(!valid_path("a/../b"));
        assert!(!valid_path("a//b"));
        assert!(!valid_path("a\0b"));
        assert!(valid_path(&"中".repeat(85)));
        assert!(!valid_path(&"中".repeat(86)));
        let base = vec!["a".repeat(255); 15].join("/");
        assert!(valid_path(&format!("{base}/file.txt")));
        assert!(!valid_path(&format!("{base}/{}/file.txt", "b".repeat(255))));
    }
}

#[cfg(test)]
mod job_tests {
    use super::*;
    #[test]
    fn business_errors_and_retry_metadata_use_only_canonical_values() {
        assert_eq!(business_code(&json!("upload_rejected")), "upload_rejected");
        for value in [
            json!("Upper"),
            json!("auth.csrf_rejected"),
            json!("with-dash"),
            json!("a".repeat(65)),
            json!(null),
            json!(0),
        ] {
            assert!(business_code(&value).is_empty());
        }
        for value in [
            "retry",
            "retry_with_new_id",
            "resume_upload",
            "query_job",
            "query_upload",
            "refresh_target",
        ] {
            assert_eq!(recovery_advice(&json!(value)), value);
        }
        assert!(recovery_advice(&json!("retry_always")).is_empty());
        assert_eq!(
            body_retry_after(&json!(crate::model::MAX_SAFE_INTEGER)),
            Some(crate::model::MAX_SAFE_INTEGER)
        );
        for value in [
            json!(crate::model::MAX_SAFE_INTEGER + 1),
            json!(-1),
            json!(0.5),
            json!("1"),
        ] {
            assert_eq!(body_retry_after(&value), None);
        }
    }
    #[test]
    fn job_queries_admit_only_canonical_uuid_paths() {
        assert!(canonical_uuid("00000000-0000-4000-8000-000000000010"));
        for id in [
            "../rename",
            "00000000-0000-4000-8000-00000000001",
            "00000000-0000-4000-8000-00000000001A",
            "00000000_0000-4000-8000-000000000010",
        ] {
            assert!(!canonical_uuid(id));
        }
    }
    #[test]
    fn a_terminal_failed_job_proves_noncommit_without_granting_overwrite_authority() {
        let mut error = Error {
            unknown: true,
            target_revision: Some("a".repeat(64)),
            ..Error::default()
        };
        settle_failed_job(
            &mut error,
            &json!({"kind":"failed","status":409,"code":"path_exists"}),
        );
        assert!(!error.unknown);
        assert!(error.trusted(409, "path_exists"));
        assert!(error.target_revision.is_none());
        let mut unknown = Error {
            unknown: true,
            ..Error::default()
        };
        for kind in ["unknown", "running", "unavailable"] {
            settle_failed_job(
                &mut unknown,
                &json!({"kind":kind,"status":500,"code":"path_exists"}),
            );
            assert!(unknown.unknown);
        }
    }
    #[test]
    fn csrf_codes_require_the_current_platform_envelope_and_json_media_type() {
        let envelope =
            json!({"code":"auth.csrf_rejected","message":"Request rejected","retryable":false});
        assert_eq!(
            platform_error_code(&envelope, "application/json; charset=utf-8"),
            Some("auth.csrf_rejected")
        );
        assert_eq!(
            platform_error_code(&envelope, "application/problem+json"),
            None
        );
        assert_eq!(platform_error_code(&envelope, "text/html"), None);
        assert_eq!(
            platform_error_code(&json!({"code":"auth.csrf_rejected"}), "application/json"),
            None
        );
        let mut alias = envelope;
        alias["error"] = json!({"code":"auth.csrf_rejected"});
        assert_eq!(platform_error_code(&alias, "application/json"), None);
    }
    #[test]
    fn job_status_binding_rejects_contradictions_and_invalid_http_states() {
        let id = "00000000-0000-4000-8000-000000000010";
        let mut p = json!({"job_id":id,"state":"succeeded","http_status":204});
        assert!(parse_job(&p, id, Some(id), Some("succeeded")).is_some());
        assert!(parse_job(&p, id, Some(id), Some("unknown")).is_none());
        assert!(parse_job(&p, id, Some("another"), Some("succeeded")).is_none());
        p["http_status"] = json!(500);
        assert!(parse_job(&p, id, Some(id), Some("succeeded")).is_none());
        p["state"] = json!("running");
        assert!(parse_job(&p, id, Some(id), Some("running")).is_none());
        p.as_object_mut().unwrap().remove("http_status");
        assert!(parse_job(&p, id, Some(id), Some("running")).is_some());
        p["state"] = json!("failed");
        assert!(parse_job(&p, id, Some(id), Some("failed")).is_none());
    }
}
