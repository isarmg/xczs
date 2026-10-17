//! Browser upload lifecycle. File handles live only in this document; refresh
//! never resumes a different file merely because its metadata happens to match.
use xcss_web::component::{component_props_builder, component_view};

use crate::{
    http::{self, Error, Raw},
    model::{self, Dialog, Model},
    operations,
    protocol::{self, Phase, State},
};
use futures::channel::oneshot;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{AbortController, Blob, File, FileList, Headers, ProgressEvent, XmlHttpRequest};
use xcss_web::i18n::t;
use xcss_web::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: u64,
    pub batch: u64,
    pub name: String,
    pub path: String,
    pub size: u64,
    pub upload_id: String,
    pub offset: u64,
    pub target_revision: Option<String>,
    pub state: String,
    pub phase: String,
    pub message: String,
    pub progress: String,
    pub speed: String,
    pub recovery: String,
    pub ready_at: f64,
    pub visible: bool,
    pub missing_retry: bool,
    pub dispatched: bool,
    pub retired: u64,
    pub recovering_unknown: bool,
}
impl Row {
    fn terminal(&self) -> bool {
        matches!(
            self.state.as_str(),
            "completed" | "failed" | "unknown" | "cancelled"
        )
    }
}
#[derive(Clone, Debug, Default)]
struct QueueState {
    rows: Vec<Row>,
    unresolved: HashSet<u64>,
    reserved: usize,
    next_id: u64,
    next_batch: u64,
    running: usize,
    paused: String,
    message: String,
    evicted: usize,
    terminal_seq: u64,
}
struct BrowserData {
    file: File,
    xhr: Option<XmlHttpRequest>,
    controller: Option<AbortController>,
}
thread_local! {
    static UI:RefCell<Option<RwSignal<QueueState>>>=const{RefCell::new(None)};
    static FILES:RefCell<HashMap<u64,Rc<RefCell<BrowserData>>>>=RefCell::new(HashMap::new());
}
fn ui() -> RwSignal<QueueState> {
    UI.with(|s| s.borrow().expect("upload queue mounted"))
}
fn row(id: u64) -> Option<Row> {
    ui().with_untracked(|s| s.rows.iter().find(|r| r.id == id).cloned())
}
fn update(id: u64, f: impl FnOnce(&mut Row)) {
    ui().update(|s| {
        if let Some(row) = s.rows.iter_mut().find(|r| r.id == id) {
            f(row);
        }
    });
}
fn data(id: u64) -> Option<Rc<RefCell<BrowserData>>> {
    FILES.with(|f| f.borrow().get(&id).cloned())
}
fn pause_auth_state(state: &mut QueueState) {
    if state.paused == "auth" {
        return;
    }
    state.paused = "auth".into();
    state.message = t(
        "会话或页面已失效。请刷新页面后重新选择文件。",
        "Your session or this page is no longer valid. Refresh the page and select the files again.",
    );
}
fn limit_message() -> String {
    t(
        "最多同时等待 512 个上传。请等待上传完成后再添加或重试。",
        "At most 512 uploads may be pending at once. Wait for pending uploads to finish before adding or retrying more.",
    )
}
fn pending(s: &QueueState) -> usize {
    s.rows.iter().filter(|r| !r.terminal()).count() + s.reserved
}
pub fn select(model: Model, files: Option<FileList>) {
    let Some(files) = files else {
        return;
    };
    let count = files.length() as usize;
    if count == 0 {
        return;
    }
    let signal = ui();
    if count > 512 {
        signal.update(|s|s.message=t("每批最多选择 512 个文件，请将较大的文件夹分批选择。","Select no more than 512 files in one batch. Split larger folders into multiple selections."));
        return;
    }
    let mut entries = Vec::new();
    let mut bytes = 0;
    let mut paths = HashSet::new();
    for i in 0..files.length() {
        let Some(file) = files.get(i) else {
            return;
        };
        let relative = js_sys::Reflect::get(&file, &"webkitRelativePath".into())
            .ok()
            .and_then(|v| v.as_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| file.name());
        bytes += relative.len();
        if !http::valid_path(&relative)
            || !http::valid_path(model.path(&relative).trim_start_matches('/'))
            || relative.rsplit('/').next() != Some(file.name().as_str())
            || bytes > 256 * 1024
        {
            signal.update(|s|s.message=t("所选上传路径无效或超过每批路径限制，请分批选择。","A selected upload path is invalid or exceeds the batch path limit. Split the selection."));
            return;
        }
        if !paths.insert(relative.clone()) {
            signal.update(|s| {
                s.message = format!(
                    "{} {}",
                    t("已跳过重复上传目标", "Skipped duplicate upload target"),
                    relative
                )
            });
            continue;
        }
        entries.push((file, relative));
    }
    if signal.with_untracked(|s| !s.paused.is_empty()) {
        return;
    }
    if signal.with_untracked(|s| pending(s) + entries.len() > 512) {
        signal.update(|s| s.message = limit_message());
        return;
    }
    let reserved = entries.len();
    signal.update(|s| s.reserved += reserved);
    xcss_web::task::spawn_local(async move {
        let result = admit(model, entries, reserved).await;
        if !matches!(result, Ok(true)) {
            signal.update(|s| s.reserved = s.reserved.saturating_sub(reserved));
        }
        if let Err(e) = result {
            signal.update(|s| {
                let auth = e.auth();
                s.message = format!(
                    "{} {}",
                    t("无法核对上传目标：", "Unable to check upload destinations:"),
                    e.message
                );
                if auth {
                    pause_auth_state(s);
                    if e.status == 401 {
                        gloo_timers::callback::Timeout::new(0, http::unauthorized).forget();
                    }
                }
            });
        }
        pump(model);
    });
}
async fn admit(model: Model, entries: Vec<(File, String)>, reserved: usize) -> Result<bool, Error> {
    let signal = ui();
    let known = signal.with_untracked(|s| {
        s.rows
            .iter()
            .filter(|r| !r.terminal())
            .map(|r| r.name.clone())
            .collect::<HashSet<_>>()
    });
    let count = entries.len();
    let entries = entries
        .into_iter()
        .filter(|(_, name)| !known.contains(name))
        .collect::<Vec<_>>();
    if entries.len() != count {
        signal.update(|s| {
            s.message = t(
                "已跳过重复上传目标，请等待现有上传完成。",
                "Skipped duplicate upload targets. Wait for existing uploads to finish.",
            )
        });
    }
    if entries.is_empty() {
        return Ok(false);
    }
    let paths = entries
        .iter()
        .map(|(_, name)| model.path(name))
        .collect::<Vec<_>>();
    let targets = http::upload_preflight(&paths).await?;
    if signal.with_untracked(|s| !s.paused.is_empty()) {
        return Ok(false);
    }
    let conflicts = targets
        .iter()
        .filter(|t| t.exists && t.replaceable)
        .map(|t| t.path.clone())
        .collect::<Vec<_>>();
    let choice = if conflicts.is_empty() {
        "confirm".into()
    } else {
        operations::ask(model,Dialog{title:t("已存在的上传目标","Existing upload destinations"),message:format!("{} {}",conflicts.join(", "),t("已存在。仅在核对后未变化时覆盖，或跳过冲突，或取消整批。","already exist. Overwrite only if unchanged since this check, skip conflicts, or cancel the batch.")),confirm:t("覆盖","Overwrite"),alternate:t("跳过冲突","Skip conflicts"),cancel:t("取消上传","Cancel upload"),danger:true,..Dialog::default()}).await.0
    };
    if choice == "cancel" {
        return Ok(false);
    }
    let batch = signal.with_untracked(|s| s.next_batch);
    signal.update(|s| s.next_batch += 1);
    signal.update(|s| s.reserved = s.reserved.saturating_sub(reserved));
    let mut ids = Vec::new();
    for ((file, name), target) in entries.into_iter().zip(targets) {
        if !target.replaceable || target.exists && choice != "confirm" {
            continue;
        }
        if signal.with_untracked(|s| s.rows.iter().any(|r| r.name == name && !r.terminal())) {
            signal.update(|s| {
                s.message = format!(
                    "{} {name}",
                    t("已跳过重复上传目标", "Skipped duplicate upload target")
                )
            });
            continue;
        }
        let id = signal.with_untracked(|s| s.next_id);
        let row = Row {
            id,
            batch,
            name: name.clone(),
            path: model.path(&name),
            size: file.size() as u64,
            upload_id: http::uuid(),
            offset: 0,
            target_revision: target.revision,
            state: "queued".into(),
            phase: "new".into(),
            message: t("等待中", "Waiting"),
            progress: String::new(),
            speed: String::new(),
            recovery: String::new(),
            ready_at: 0.,
            visible: false,
            missing_retry: false,
            dispatched: false,
            retired: 0,
            recovering_unknown: false,
        };
        FILES.with(|f| {
            f.borrow_mut().insert(
                id,
                Rc::new(RefCell::new(BrowserData {
                    file,
                    xhr: None,
                    controller: None,
                })),
            );
        });
        signal.update(|s| {
            s.next_id += 1;
            s.rows.push(row);
        });
        ids.push(id);
    }
    if choice == "alternate" {
        signal.update(|s| {
            s.message = t(
                "已跳过冲突的上传目标。",
                "Skipped the conflicting upload destinations.",
            )
        });
    }
    // Every destination and pending slot is claimed before yielding to DOM
    // chunking, so a tail entry is protected from duplicate recovery clicks.
    for chunk in ids.chunks(50) {
        for id in chunk {
            update(*id, |r| r.visible = true);
        }
        pump(model);
        wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(
            &wasm_bindgen::JsValue::UNDEFINED,
        ))
        .await
        .ok();
        gloo_timers::future::TimeoutFuture::new(0).await;
    }
    Ok(true)
}
fn pump(model: Model) {
    let signal = ui();
    if signal.with_untracked(|s| s.running > 0 || !s.paused.is_empty()) {
        return;
    }
    let next = signal.with_untracked(|s| {
        s.rows
            .iter()
            .find(|r| r.visible && r.state == "queued")
            .map(|r| r.id)
    });
    if let Some(id) = next {
        signal.update(|s| s.running += 1);
        update(id, |r| {
            r.state = "running".into();
        });
        xcss_web::task::spawn_local(async move {
            run(model, id).await;
        });
    }
}
fn has_unresolved_unknown(rows: &[Row]) -> bool {
    rows.iter()
        .any(|r| r.state == "unknown" || r.recovering_unknown)
}
fn settle_pause(state: &mut QueueState, unknown: bool) {
    if unknown && state.paused != "auth" {
        state.paused = "unknown".into();
        state.message = t(
            "上传结果不确定，剩余队列已暂停。请刷新文件夹后再选择文件。",
            "An upload result is unknown. The remaining upload queue is paused; refresh the folder before selecting files again.",
        );
    } else if state.unresolved.is_empty()
        && !has_unresolved_unknown(&state.rows)
        && state.paused == "unknown"
    {
        state.paused.clear();
        state.message.clear();
    }
}
fn retired_history_ids(rows: &[Row], limit: usize) -> HashSet<u64> {
    let mut history = rows
        .iter()
        .filter(|r| r.terminal())
        .map(|r| (r.retired, r.id))
        .collect::<Vec<_>>();
    history.sort_unstable();
    let count = history.len().saturating_sub(limit);
    history.into_iter().take(count).map(|(_, id)| id).collect()
}
fn history_message(evicted: usize, visible: usize) -> String {
    if xcss_web::i18n::locale() == "en" {
        format!(
            "{evicted} older upload result{} hidden; showing the most recent {visible}.",
            if evicted == 1 { "" } else { "s" }
        )
    } else {
        format!("已隐藏 {evicted} 条较早的上传结果；显示最近 {visible} 条。")
    }
}
fn trim_history(s: &mut QueueState) {
    let terminal = s.rows.iter().filter(|r| r.terminal()).count();
    if terminal <= 200 {
        return;
    }
    let remove = retired_history_ids(&s.rows, 200);
    let restore = http::document()
        .active_element()
        .and_then(|e| e.closest(".uploader").ok().flatten())
        .and_then(|e| e.get_attribute("data-upload-id"))
        .and_then(|s| s.parse::<u64>().ok())
        .is_some_and(|id| remove.contains(&id));
    s.rows.retain(|r| !remove.contains(&r.id));
    s.evicted += remove.len();
    FILES.with(|f| f.borrow_mut().retain(|id, _| !remove.contains(id)));
    if restore {
        gloo_timers::callback::Timeout::new(0, || {
            if let Some(element) = http::document()
                .query_selector(".uploader .cell-name a")
                .ok()
                .flatten()
                .or_else(|| {
                    http::document()
                        .query_selector(".upload-history-status")
                        .ok()
                        .flatten()
                })
                .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
            {
                if element.class_list().contains("upload-history-status") {
                    let _ = element.set_attribute("tabindex", "-1");
                }
                let _ = element.focus();
            }
        })
        .forget();
    }
}
fn finish(model: Model, id: u64, state: &str, message: String, recovery: &str, delay: Option<u64>) {
    let focused = http::document().active_element().is_some_and(|e| {
        e.closest(&format!("#uploadStatus{id}"))
            .ok()
            .flatten()
            .is_some()
    });
    let signal = ui();
    signal.update(|s| {
        s.terminal_seq += 1;
        let seq = s.terminal_seq;
        if let Some(r) = s.rows.iter_mut().find(|r| r.id == id) {
            r.state = state.into();
            r.phase = state.into();
            r.message = message;
            r.recovery = recovery.into();
            r.ready_at = js_sys::Date::now() + delay.unwrap_or(0) as f64 * 1000.;
            r.dispatched = false;
            r.retired = seq;
            r.recovering_unknown = state == "unknown";
        }
        s.running = s.running.saturating_sub(1);
        if state == "unknown" {
            s.unresolved.insert(id);
        } else {
            s.unresolved.remove(&id);
        }
        settle_pause(s, state == "unknown");
        trim_history(s);
    });
    if focused {
        gloo_timers::callback::Timeout::new(0, move || {
            let retry = format!("#upload{id} .retry-btn:not([hidden]):not(:disabled)");
            let name = format!("#upload{id} .cell-name a");
            if let Some(element) = http::document()
                .query_selector(&retry)
                .ok()
                .flatten()
                .or_else(|| http::document().query_selector(&name).ok().flatten())
                .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
            {
                let _ = element.focus();
            }
        })
        .forget();
    }
    if state == "completed" {
        model.dir_exists.set(true);
        model.invalidate(false);
    } else if state == "unknown" {
        model.invalidate(true);
    }
    pump(model);
}
pub fn cancel(model: Model, id: u64) {
    let Some(r) = row(id) else {
        return;
    };
    if r.state == "queued" {
        ui().update(|s| {
            s.terminal_seq += 1;
            let seq = s.terminal_seq;
            if let Some(r) = s.rows.iter_mut().find(|r| r.id == id) {
                r.state = "cancelled".into();
                r.phase = "cancelled".into();
                r.message = t("上传已取消", "Upload cancelled");
                r.recovery.clear();
                r.retired = seq;
            }
            trim_history(s);
        });
        gloo_timers::callback::Timeout::new(0, move || {
            if let Some(link) = http::document()
                .query_selector(&format!("#upload{id} .cell-name a"))
                .ok()
                .flatten()
                .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
            {
                let _ = link.focus();
            }
        })
        .forget();
        pump(model);
        return;
    }
    if r.state != "running" {
        return;
    }
    if let Some(data) = data(id) {
        let d = data.borrow();
        if let Some(xhr) = &d.xhr {
            let _ = xhr.abort();
        }
        if let Some(c) = &d.controller {
            c.abort();
        }
    }
}
fn cancel_batch(id: u64) {
    let Some(r) = row(id) else {
        return;
    };
    ui().update(|s|{
    let mut count=0;let mut seq=s.terminal_seq;let mut unmaterialized=HashSet::new();
    for other in &mut s.rows{if other.batch==r.batch&&other.id!=id&&other.state=="queued"{count+=1;if !other.visible{unmaterialized.insert(other.id);continue;}other.state="cancelled".into();other.phase="cancelled".into();other.message=t("上传已取消","Upload cancelled");seq+=1;other.retired=seq;}}
    s.terminal_seq=seq;s.rows.retain(|r|!unmaterialized.contains(&r.id));FILES.with(|files|files.borrow_mut().retain(|id,_|!unmaterialized.contains(id)));trim_history(s);
    s.message=if xcss_web::i18n::locale()=="en"{format!("Cancelled {count} remaining queued upload{}. Uploads already in progress were not interrupted.",if count==1{""}else{"s"})}else{format!("已取消剩余 {count} 个排队上传。进行中的上传未被中断。")};
});
}
fn recover(model: Model, id: u64) {
    let Some(r) = row(id) else {
        return;
    };
    if r.recovery.is_empty() || js_sys::Date::now() < r.ready_at {
        return;
    }
    let signal = ui();
    if signal.with_untracked(|s| pending(s) >= 512) {
        signal.update(|s| s.message = limit_message());
        return;
    }
    if signal.with_untracked(|s| {
        s.rows
            .iter()
            .any(|other| other.id != id && !other.terminal() && other.name == r.name)
    }) {
        signal.update(|s|s.message=if xcss_web::i18n::locale()=="en"{format!("Another upload already targets {}. Wait for it to finish before retrying this upload.",r.name)}else{format!("已有其他上传以 {} 为目标，请等待其完成后再重试。",r.name)});
        return;
    }
    if signal.with_untracked(|s| !s.paused.is_empty())
        && !(r.state == "unknown" && r.recovery == "query_upload")
    {
        return;
    }
    if r.state == "unknown" {
        signal.update(|s| s.running += 1);
        update(id, |r| {
            r.state = "running".into();
            r.recovering_unknown = true;
        });
        xcss_web::task::spawn_local(async move {
            run(model, id).await;
        });
    } else {
        update(id, |r| {
            r.state = "queued".into();
            r.recovering_unknown = false;
            r.message = t("等待重试", "Waiting to retry");
        });
        pump(model);
    }
    gloo_timers::callback::Timeout::new(0, move || {
        if let Some(button) = http::document()
            .query_selector(&format!("#upload{id} .upload-cancel"))
            .ok()
            .flatten()
            .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let _ = button.focus();
        }
    })
    .forget();
}
enum Next {
    Send {
        resume: bool,
    },
    Check {
        retry: bool,
        was_unknown: bool,
    },
    Done {
        state: &'static str,
        message: String,
        recovery: &'static str,
        delay: Option<u64>,
    },
}
fn done(state: &'static str, message: String, recovery: &'static str, delay: Option<u64>) -> Next {
    Next::Done {
        state,
        message,
        recovery,
        delay,
    }
}
async fn run(model: Model, id: u64) {
    let Some(initial) = row(id) else {
        return;
    };
    let mut next = if initial.recovery.is_empty() {
        Next::Send { resume: false }
    } else {
        Next::Check {
            retry: initial.recovery == "retry",
            was_unknown: initial.recovering_unknown,
        }
    };
    update(id, |r| r.recovery.clear());
    loop {
        next = match next {
            Next::Send { resume } => {
                let result = transfer(id, resume).await;
                match result {
                    Ok(raw) => {
                        response(
                            model,
                            id,
                            raw,
                            if resume { Phase::Resume } else { Phase::Fresh },
                        )
                        .await
                    }
                    Err(e) => {
                        if e.auth() {
                            ui().update(pause_auth_state);
                            done("failed", e.message, "", None)
                        } else {
                            done(
                                if e.unknown { "unknown" } else { "failed" },
                                e.message,
                                if e.unknown { "query_upload" } else { "retry" },
                                None,
                            )
                        }
                    }
                }
            }
            Next::Check { retry, was_unknown } => checkpoint(model, id, retry, was_unknown).await,
            Next::Done {
                state,
                message,
                recovery,
                delay,
            } => {
                finish(model, id, state, message, recovery, delay);
                break;
            }
        };
    }
}
async fn checkpoint(model: Model, id: u64, retry: bool, was_unknown: bool) -> Next {
    let Some(r) = row(id) else {
        return done("failed", "Missing upload".into(), "", None);
    };
    let controller = AbortController::new().unwrap();
    if let Some(data) = data(id) {
        data.borrow_mut().controller = Some(controller.clone());
    }
    update(id, |r| {
        r.phase = "checking".into();
        r.dispatched = false;
        r.message = t("正在核对续传状态…", "Checking resume status…");
    });
    let result = http::raw(
        &http::file_url(&r.path),
        "HEAD",
        vec![(http::UP_ID.into(), r.upload_id.clone())],
        None,
        Some(controller.clone()),
        30_000,
        false,
    )
    .await;
    if let Some(data) = data(id) {
        data.borrow_mut().controller = None;
    }
    let raw = match result {
        Ok(r) => r,
        Err(e) => {
            return done(
                if was_unknown { "unknown" } else { "failed" },
                if controller.signal().aborted() {
                    t("上传已取消", "Upload cancelled")
                } else {
                    e.message
                },
                "query_upload",
                None,
            );
        }
    };
    if raw.status == 401 {
        gloo_timers::callback::Timeout::new(0, http::unauthorized).forget();
        ui().update(pause_auth_state);
        return done(
            "failed",
            t(
                "会话已失效，请刷新页面。",
                "Your session has expired. Refresh the page.",
            ),
            "",
            None,
        );
    }
    let Some(bound) = protocol::classify(
        Phase::Checkpoint,
        raw.status,
        |n| raw.header(n),
        &r.upload_id,
        r.size,
    ) else {
        return done(
            "unknown",
            t(
                "服务器返回的上传检查点不一致。",
                "The server returned an inconsistent upload checkpoint.",
            ),
            "query_upload",
            None,
        );
    };
    match bound.state {
        State::Committed => done("completed", t("上传完成", "Upload complete"), "", None),
        State::NotSeen | State::Rejected => {
            if retry {
                update(id, |r| {
                    r.upload_id = http::uuid();
                    r.offset = 0;
                    r.missing_retry = false;
                });
                Next::Send { resume: false }
            } else {
                done(
                    "failed",
                    t(
                        "上传会话无法续传，请启动新的上传会话",
                        "The upload session cannot be resumed; start a new upload session",
                    ),
                    "retry",
                    None,
                )
            }
        }
        State::Running => {
            let offset = bound.offset.unwrap();
            update(id, |r| r.offset = offset);
            if retry || offset == r.size {
                Next::Send { resume: true }
            } else {
                done(
                    "failed",
                    t(
                        "上传仍可从已确认的检查点续传",
                        "Upload remains resumable from the confirmed checkpoint",
                    ),
                    "retry",
                    None,
                )
            }
        }
        State::AwaitingConfirmation => {
            if bound.offset != Some(r.size) {
                return done(
                    "unknown",
                    t(
                        "服务器返回的覆盖检查点无效。",
                        "The server returned an invalid overwrite checkpoint.",
                    ),
                    "query_upload",
                    None,
                );
            }
            model.refresh_required();
            if raw.header(http::TARGET).is_none()
                && raw.header("X-Xczs-Target-Replaceable").as_deref() == Some("true")
            {
                update(id, |r| {
                    r.phase = "cleanup".into();
                    r.message = t("正在清理暂存上传…", "Cleaning up staged upload…");
                });
                if let Some(result) = cleanup_staged(id, true).await {
                    return result;
                }
                update(id, |r| {
                    r.upload_id = http::uuid();
                    r.offset = 0;
                    r.target_revision = None;
                    r.missing_retry = false;
                });
                return Next::Send { resume: false };
            }
            resolve_conflict(model, id, &raw, true).await
        }
        State::Unknown => {
            if matches!(raw.status, 429 | 503) {
                done(
                    "unknown",
                    if raw.status == 429 {
                        t(
                            "上传状态查询繁忙，请再次核对状态后再上传",
                            "Upload status queries are temporarily busy. Check the upload status again before trying to upload",
                        )
                    } else {
                        t(
                            "上传状态暂不可用，请再次核对状态后再上传",
                            "Upload status is temporarily unavailable. Check the upload status again before trying to upload",
                        )
                    },
                    "query_upload",
                    raw.header("Retry-After")
                        .as_deref()
                        .and_then(protocol::unsigned),
                )
            } else {
                done(
                    "unknown",
                    t(
                        "服务器记录的发布结果不确定。请刷新文件夹核对目标后再选择文件",
                        "The server recorded an uncertain publication outcome. Refresh the folder and inspect the target before selecting the file again",
                    ),
                    "",
                    None,
                )
            }
        }
        _ => done("unknown", http::unknown_message(), "query_upload", None),
    }
}
async fn response(model: Model, id: u64, raw: Raw, phase: Phase) -> Next {
    let r = row(id).unwrap();
    if raw.status == 401 {
        ui().update(pause_auth_state);
        gloo_timers::callback::Timeout::new(0, http::unauthorized).forget();
        return done(
            "failed",
            t(
                "会话已失效，请刷新页面。",
                "Your session has expired. Refresh the page.",
            ),
            "",
            None,
        );
    }
    let bound = protocol::classify(phase, raw.status, |n| raw.header(n), &r.upload_id, r.size);
    if bound.as_ref().is_some_and(|b| b.state == State::Committed) {
        return done("completed", t("上传完成", "Upload complete"), "", None);
    }
    let unknown = bound
        .as_ref()
        .is_none_or(|b| matches!(b.state, State::Unknown | State::Running));
    let e = raw.upload_error(unknown);
    if e.auth() {
        ui().update(pause_auth_state);
        return done("failed", e.message, "", None);
    }
    if e.problem_status != 0 && e.problem_status != raw.status {
        return done(
            "unknown",
            e.message,
            if e.recovery == "query_upload" {
                "query_upload"
            } else {
                ""
            },
            e.retry_after,
        );
    }
    let Some(bound) = bound else {
        return done(
            "unknown",
            inconsistent_response(raw.status),
            if e.recovery == "query_upload" {
                "query_upload"
            } else {
                ""
            },
            e.retry_after,
        );
    };
    let running_conflict =
        bound.state == State::Running && phase == Phase::Resume && bound.offset == Some(r.offset);
    let target_change = (matches!(bound.state, State::NotStarted | State::AwaitingConfirmation)
        || running_conflict)
        && raw.status == 409
        && e.problem_status == 409
        && bound.length == Some(r.size)
        && (bound.state != State::AwaitingConfirmation || bound.offset == Some(r.size));
    if target_change {
        let staged = bound.state == State::AwaitingConfirmation;
        if e.code == "destination_exists"
            && raw
                .header(http::TARGET)
                .as_ref()
                .is_some_and(|v| protocol::revision(v))
            && matches!(
                raw.header("X-Xczs-Target-Replaceable").as_deref(),
                Some("true" | "false")
            )
        {
            model.refresh_required();
            return resolve_conflict(model, id, &raw, staged).await;
        }
        let reset_stage = e.code == "upload_metadata_preservation_refused" && staged;
        let missing = e.code == "upload_target_changed"
            && raw.header("X-Xczs-Target-Replaceable").as_deref() == Some("true")
            && raw.header(http::TARGET).is_none();
        if reset_stage || missing {
            model.refresh_required();
            if staged {
                update(id, |r| {
                    r.phase = "cleanup".into();
                    r.message = t("正在清理暂存上传…", "Cleaning up staged upload…");
                });
                if let Some(result) = cleanup_staged(id, true).await {
                    return result;
                }
                update(id, |r| {
                    r.upload_id = http::uuid();
                    r.offset = 0;
                    r.target_revision = None;
                    r.missing_retry = false;
                });
                return Next::Send { resume: false };
            }
            if r.missing_retry {
                return done(
                    "failed",
                    t(
                        "上传目标持续变化，请先核对状态再重试",
                        "The upload destination kept changing; check its status before trying again",
                    ),
                    "query_upload",
                    None,
                );
            }
            update(id, |r| {
                r.missing_retry = true;
                r.target_revision = None;
            });
            return Next::Send {
                resume: phase == Phase::Resume,
            };
        }
    }
    if bound.state == State::AwaitingConfirmation
        && bound.offset == Some(r.size)
        && e.problem_status == raw.status
        && e.recovery == "query_upload"
    {
        return Next::Check {
            retry: false,
            was_unknown: true,
        };
    }
    match bound.state {
        State::Unknown => done(
            "unknown",
            if e.message.is_empty() {
                http::unknown_message()
            } else {
                e.message
            },
            if e.recovery == "query_upload" {
                "query_upload"
            } else {
                ""
            },
            e.retry_after,
        ),
        State::Running | State::NotStarted | State::Rejected | State::NotSeen => {
            let recovery = match (e.recovery.as_str(), bound.state) {
                ("query_upload", _) => "query_upload",
                ("retry", State::NotStarted)
                | ("retry_with_new_id", State::Rejected | State::NotSeen)
                | ("resume_upload", State::Running) => "retry",
                _ => "",
            };
            done("failed", e.message, recovery, e.retry_after)
        }
        _ => done(
            "unknown",
            inconsistent_response(raw.status),
            if e.recovery == "query_upload" {
                "query_upload"
            } else {
                ""
            },
            e.retry_after,
        ),
    }
}
fn inconsistent_response(status: u16) -> String {
    format!(
        "{} (HTTP {status}). {}",
        t(
            "服务器返回的上传响应不一致",
            "The server returned an inconsistent upload response"
        ),
        http::unknown_message()
    )
}
async fn resolve_conflict(model: Model, id: u64, raw: &Raw, staged: bool) -> Next {
    let r = row(id).unwrap();
    let revision = raw.header(http::TARGET).filter(|v| protocol::revision(v));
    let replaceable = raw.header("X-Xczs-Target-Replaceable");
    if revision.is_none() || !matches!(replaceable.as_deref(), Some("true" | "false")) {
        return done(
            "unknown",
            t(
                "覆盖目标缺少可信的修订信息。",
                "The overwrite destination lacks a trusted revision.",
            ),
            "query_upload",
            None,
        );
    }
    if replaceable.as_deref() == Some("false") {
        if staged && let Some(result) = cleanup_staged(id, false).await {
            return result;
        }
        return done(
            "cancelled",
            t(
                "已跳过，因为目标无法替换。",
                "Skipped because the destination cannot be replaced.",
            ),
            "",
            None,
        );
    }
    update(id, |r| {
        r.phase = "awaiting-confirmation".into();
        r.message = t("等待覆盖确认", "Waiting for overwrite confirmation");
    });
    let(choice,_)=operations::ask(model,Dialog{title:t("上传目标已变更","Upload destination changed"),message:format!("{}: {}",r.name,if staged{t("数据已暂存。请选择覆盖当前目标、跳过此文件或取消剩余队列。","The uploaded data is staged. Overwrite the current destination, skip this file, or cancel the remaining queued files.")}else{t("发送数据前目标已变化。请选择覆盖当前目标、跳过此文件或取消剩余队列。","The destination now exists or changed before its data was sent. Overwrite the current destination, skip this file, or cancel the remaining queued files.")}),confirm:t("覆盖","Overwrite"),alternate:t("跳过文件","Skip file"),cancel:t("取消剩余队列","Cancel remaining"),danger:true,..Dialog::default()}).await;
    if choice == "confirm" {
        update(id, |r| {
            r.target_revision = revision;
            r.missing_retry = false;
            if staged {
                r.offset = r.size;
            }
        });
        return Next::Send {
            resume: staged || r.offset > 0,
        };
    }
    if choice == "cancel" {
        cancel_batch(id);
    }
    if staged && let Some(result) = cleanup_staged(id, false).await {
        return result;
    }
    done(
        "cancelled",
        t("已跳过：目标已存在", "Skipped: destination exists"),
        "",
        None,
    )
}

/// Cleanup is a mutation, followed by at most one read-only reconciliation.
/// Empty-file candidate creation deliberately retains its stricter discard path.
async fn cleanup_staged(id: u64, restart: bool) -> Option<Next> {
    let r = row(id)?;
    update(id, |r| {
        r.phase = "cleanup".into();
        r.message = t("正在清理暂存上传…", "Cleaning up staged upload…");
    });
    let auth = |e: Error| {
        ui().update(pause_auth_state);
        if e.status == 401 {
            gloo_timers::callback::Timeout::new(0, http::unauthorized).forget();
        }
        Some(done("failed", e.message, "", None))
    };
    match http::json_request(
        "/__xczs__/api/upload/discard",
        "POST",
        Some(serde_json::json!({"path":r.path,"upload_id":r.upload_id})),
        vec![],
        true,
    )
    .await
    {
        Ok(raw) => {
            if protocol::classify(
                Phase::Discard,
                raw.status,
                |n| raw.header(n),
                &r.upload_id,
                r.size,
            )
            .is_some()
            {
                return None;
            }
            let error = raw.upload_error(true);
            if error.auth() {
                return auth(error);
            }
        }
        Err(e) if e.auth() => return auth(e),
        Err(_) => {}
    }
    match http::raw(
        &http::file_url(&r.path),
        "HEAD",
        vec![(http::UP_ID.into(), r.upload_id.clone())],
        None,
        None,
        30_000,
        false,
    )
    .await
    {
        Ok(raw) => {
            if raw.status == 401 {
                return auth(raw.upload_error(false));
            }
            let e = raw.upload_error(false);
            if e.auth() {
                return auth(e);
            }
            if let Some(bound) = protocol::classify(
                Phase::Checkpoint,
                raw.status,
                |n| raw.header(n),
                &r.upload_id,
                r.size,
            ) {
                match bound.state {
                    State::Rejected | State::NotSeen => return None,
                    State::Committed => {
                        return Some(done(
                            "completed",
                            t("上传完成", "Upload complete"),
                            "",
                            None,
                        ));
                    }
                    _ => {}
                }
            }
        }
        Err(e) if e.auth() => return auth(e),
        Err(_) => {}
    }
    Some(done(
        "failed",
        if restart {
            t(
                "目标未被覆盖，但无法确认暂存上传是否已清理；请先核对状态再重试",
                "The destination was not overwritten, but staged-upload cleanup could not be confirmed; check its status before retrying",
            )
        } else {
            t(
                "文件未被覆盖，但无法确认暂存上传是否已清理；服务器将自动使其过期",
                "File was not overwritten, but cleanup of its staged upload could not be confirmed; the server will expire it automatically",
            )
        },
        if restart { "retry" } else { "" },
        None,
    ))
}

async fn transfer(id: u64, resume: bool) -> Result<Raw, Error> {
    let r = row(id).ok_or_else(|| Error::message("Missing upload"))?;
    let file = data(id)
        .ok_or_else(|| Error::message("Missing file handle"))?
        .borrow()
        .file
        .clone();
    let xhr = XmlHttpRequest::new().map_err(|_| Error::network(false))?;
    xhr.open(
        if resume { "PATCH" } else { "PUT" },
        &http::file_url(&r.path),
    )
    .map_err(|_| Error::network(false))?;
    for (name, value) in [
        (http::CSRF, http::token()),
        (http::UP_ID, r.upload_id.clone()),
        (http::UP_LENGTH, r.size.to_string()),
        (http::UP_OVERWRITE, r.target_revision.is_some().to_string()),
    ] {
        xhr.set_request_header(name, &value)
            .map_err(|_| Error::network(false))?;
    }
    if resume {
        xhr.set_request_header(http::UP_OFFSET, &r.offset.to_string())
            .map_err(|_| Error::network(false))?;
    }
    if let Some(rev) = &r.target_revision {
        xhr.set_request_header(http::TARGET, rev)
            .map_err(|_| Error::network(false))?;
    }
    let body: Blob = if resume {
        file.slice_with_f64(r.offset as f64)
            .map_err(|_| Error::network(false))?
    } else {
        file.unchecked_into()
    };
    let (sender, receiver) = oneshot::channel::<Result<Raw, Error>>();
    let sender = Rc::new(RefCell::new(Some(sender)));
    let started = js_sys::Date::now();
    let last_progress = Rc::new(RefCell::new(started));
    let commit_started = Rc::new(RefCell::new(if body.size() == 0. {
        Some(started)
    } else {
        None::<f64>
    }));
    update(id, |r| {
        r.phase = "transferring".into();
        r.message = t("正在上传", "Uploading");
        r.progress = "0% --:--:--".into();
        r.dispatched = true;
    });
    if let Some(data) = data(id) {
        data.borrow_mut().xhr = Some(xhr.clone());
    }
    // XHR.abort synchronously emits readystatechange. These callbacks can be
    // reentered while cutting off an oversized body, so they must implement Fn
    // rather than wasm-bindgen's exclusively borrowed FnMut trampoline.
    type Listener = (
        web_sys::EventTarget,
        &'static str,
        Closure<dyn Fn(web_sys::Event)>,
    );
    let mut listeners: Vec<Listener> = Vec::new();
    let progress_last = last_progress.clone();
    let progress_commit = commit_started.clone();
    let progress = Closure::<dyn Fn(web_sys::Event)>::new(move |e: web_sys::Event| {
        let e: ProgressEvent = e.unchecked_into();
        *progress_last.borrow_mut() = js_sys::Date::now();
        if progress_commit.borrow().is_some() {
            return;
        }
        let loaded = e.loaded();
        let elapsed = (js_sys::Date::now() - started).max(1.) / 1000.;
        let speed = loaded / elapsed;
        let percent = if r.size == 0 {
            100.
        } else {
            (loaded + r.offset as f64) / r.size as f64 * 100.
        };
        update(id, |r| {
            r.speed = format!("{}/s", model::size(speed as u64));
            r.progress = format!("{percent:.1}%");
        });
    });
    let upload: web_sys::EventTarget = xhr.upload().map_err(|_| Error::network(false))?.into();
    let _ = upload.add_event_listener_with_callback("progress", progress.as_ref().unchecked_ref());
    listeners.push((upload.clone(), "progress", progress));
    let commit = commit_started.clone();
    let sent = Closure::<dyn Fn(web_sys::Event)>::new(move |_| {
        *commit.borrow_mut() = Some(js_sys::Date::now());
        update(id, |r| {
            r.phase = "submitting".into();
            r.message = t(
                "数据已发送，等待服务器确认…",
                "Data sent; waiting for server confirmation…",
            );
        });
    });
    let _ = upload.add_event_listener_with_callback("load", sent.as_ref().unchecked_ref());
    listeners.push((upload, "load", sent));
    let events: web_sys::EventTarget = xhr.clone().into();
    for kind in ["readystatechange", "progress", "load", "error", "abort"] {
        let request = xhr.clone();
        let sender = sender.clone();
        let listener = Closure::<dyn Fn(web_sys::Event)>::new(move |e: web_sys::Event| {
            if sender.borrow().is_none() {
                return;
            }
            if kind == "readystatechange" {
                if request.ready_state() != 2 {
                    return;
                }
                if request.status().unwrap_or_default() == 401 {
                    if let Some(sender) = sender.borrow_mut().take() {
                        let _ = sender.send(Ok(snapshot(&request, false)));
                    }
                    let _ = request.abort();
                    return;
                }
                if request
                    .get_response_header("Content-Length")
                    .ok()
                    .flatten()
                    .as_deref()
                    .and_then(protocol::unsigned)
                    .is_none_or(|n| n <= http::ERROR_LIMIT as u64)
                {
                    return;
                }
            } else if kind == "progress" {
                let p: ProgressEvent = e.unchecked_into();
                if p.loaded() <= http::ERROR_LIMIT as f64 {
                    return;
                }
            } else if kind == "load" {
                let raw = snapshot(&request, true);
                if raw.body.len() <= http::ERROR_LIMIT {
                    if let Some(sender) = sender.borrow_mut().take() {
                        let _ = sender.send(Ok(raw));
                    }
                    return;
                }
            } else if matches!(kind, "error" | "abort") {
                if let Some(sender) = sender.borrow_mut().take() {
                    let mut error = Error::network(true);
                    if kind == "abort" {
                        error.message = format!(
                            "{} {}",
                            t(
                                "已请求取消上传，但服务器结果不确定。",
                                "Upload cancellation was requested, but the server result is unknown."
                            ),
                            http::unknown_message()
                        );
                    }
                    let _ = sender.send(Err(error));
                }
                return;
            }
            if let Some(sender) = sender.borrow_mut().take() {
                let _ = sender.send(Err(Error {
                    message: t(
                        "服务器响应超过允许的大小",
                        "The server response exceeded the allowed size",
                    ),
                    unknown: true,
                    ..Error::default()
                }));
            }
            let _ = request.abort();
        });
        let _ = events.add_event_listener_with_callback(kind, listener.as_ref().unchecked_ref());
        listeners.push((events.clone(), kind, listener));
    }
    let check_xhr = xhr.clone();
    let check_sender = sender.clone();
    let timer = gloo_timers::callback::Interval::new(1_000, move || {
        if check_sender.borrow().is_none() {
            return;
        }
        let now = js_sys::Date::now();
        let timeout = if let Some(commit) = *commit_started.borrow() {
            now - commit > 300_000.
        } else {
            now - *last_progress.borrow() > 120_000. || now - started > 86_400_000.
        };
        if timeout {
            if let Some(sender) = check_sender.borrow_mut().take() {
                let _ = sender.send(Err(Error {
                    message: http::unknown_message(),
                    unknown: true,
                    ..Error::default()
                }));
            }
            let _ = check_xhr.abort();
        }
    });
    if xhr.send_with_opt_blob(Some(&body)).is_err()
        && let Some(sender) = sender.borrow_mut().take()
    {
        let _ = sender.send(Err(Error::network(false)));
    }
    if body.size() == 0. {
        update(id, |r| {
            r.phase = "submitting".into();
            r.message = t("正在提交…", "Submitting…");
        });
    }
    let result = receiver.await.unwrap_or_else(|_| Err(Error::network(true)));
    drop(timer);
    for (target, event, listener) in listeners {
        let _ =
            target.remove_event_listener_with_callback(event, listener.as_ref().unchecked_ref());
    }
    if let Some(data) = data(id) {
        data.borrow_mut().xhr = None;
    }
    result
}
fn snapshot(xhr: &XmlHttpRequest, body: bool) -> Raw {
    let headers = Headers::new().unwrap();
    for line in xhr.get_all_response_headers().unwrap_or_default().lines() {
        if let Some((key, value)) = line.split_once(':') {
            let _ = headers.set(key.trim(), value.trim());
        }
    }
    Raw {
        status: xhr.status().unwrap_or_default(),
        headers,
        body: if body {
            xhr.response_text()
                .ok()
                .flatten()
                .unwrap_or_default()
                .into_bytes()
        } else {
            vec![]
        },
    }
}

#[component]
pub fn Queue(model: Model) -> impl IntoView {
    let signal = RwSignal::new(QueueState::default());
    UI.with(|s| *s.borrow_mut() = Some(signal));
    let tick = RwSignal::new(0u64);
    gloo_timers::callback::Interval::new(500, move || tick.update(|v| *v += 1)).forget();
    let guard = Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(
        move |e: web_sys::BeforeUnloadEvent| {
            if signal.with_untracked(|s| {
                s.running > 0
                    || s.paused.is_empty()
                        && (s.reserved > 0 || s.rows.iter().any(|r| r.state == "queued"))
            }) {
                e.prevent_default();
                e.set_return_value("");
            }
        },
    );
    let _ = http::window()
        .add_event_listener_with_callback("beforeunload", guard.as_ref().unchecked_ref());
    guard.forget();
    View::new((
        xcss_web::html::div()
            .child(
                #[allow(unused_braces)]
                {
                    IntoRender::into_render({ move || signal.get().message })
                },
            )
            .class("upload-queue-message")
            .role("alert")
            .aria_live("assertive")
            .hidden(::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                move || signal.get().message.is_empty(),
            )),
        xcss_web::html::table()
            .child((
                xcss_web::html::caption()
                    .child(
                        #[allow(unused_braces)]
                        {
                            IntoRender::into_render({
                                move || {
                                    history_message(
                                        signal.get().evicted,
                                        signal.get().rows.iter().filter(|r| r.terminal()).count(),
                                    )
                                }
                            })
                        },
                    )
                    .class("upload-history-status")
                    .role("status")
                    .aria_live("polite")
                    .hidden(::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                        move || signal.get().evicted == 0,
                    ))
                    .on(xcss_web::ev::blur, move |e| {
                        if let Some(e) = e
                            .target()
                            .and_then(|e| e.dyn_into::<web_sys::Element>().ok())
                        {
                            let _ = e.remove_attribute("tabindex");
                        }
                    }),
                xcss_web::html::thead().child(
                    #[allow(unused_braces)]
                    {
                        xcss_web::html::tr().child((
                            xcss_web::html::th()
                                .child(
                                    #[allow(unused_braces)]
                                    {
                                        IntoRender::into_render({ t("名称", "Name") })
                                    },
                                )
                                .class("cell-name")
                                .colspan("2"),
                            xcss_web::html::th()
                                .child(
                                    #[allow(unused_braces)]
                                    {
                                        IntoRender::into_render({
                                            t("上传状态", "Upload status")
                                        })
                                    },
                                )
                                .class("cell-status"),
                        ))
                    },
                ),
                xcss_web::html::tbody().child(
                    #[allow(unused_braces)]
                    {
                        {
                            #[allow(unreachable_code)]
                            #[allow(unused_mut)]
                            #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
                            component_view(
                                #[allow(clippy::needless_borrows_for_generic_args)]
                                &For,
                                {
                                    let mut props = component_props_builder(
                                            &For,
                                        )
                                        .each(
                                            #[allow(unused_braces)]
                                            {
                                                move || {
                                                    signal
                                                        .with(|s| {
                                                            s
                                                                .rows
                                                                .iter()
                                                                .filter(|r| r.visible)
                                                                .map(|r| r.id)
                                                                .collect::<Vec<_>>()
                                                        })
                                                }
                                            },
                                        )
                                        .key(#[allow(unused_braces)] { |id| *id })
                                        .children(
                                            #[allow(unused_braces)]
                                            {
                                                move |id| {
                                                    View::new({
    #[allow(unreachable_code)] #[allow(unused_mut)] #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
    component_view(
        #[allow(clippy::needless_borrows_for_generic_args)]
        &UploadRow,
        {
            let mut props = component_props_builder(&UploadRow)
                .model(#[allow(unused_braces)] { model })
                .id(#[allow(unused_braces)] { id })
                .tick(#[allow(unused_braces)] { tick })
                .signal(#[allow(unused_braces)] { signal })
                .build();
            props
        },
    )
})
                                                }
                                            },
                                        )
                                        .build();
                                    props
                                },
                            )
                        }
                    },
                ),
            ))
            .class("uploaders-table xcss-table")
            .aria_label(::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                t("上传队列", "Upload queue"),
            ))
            .hidden(::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                move || signal.get().rows.is_empty(),
            )),
    ))
}
#[component]
fn UploadRow(
    model: Model,
    id: u64,
    tick: RwSignal<u64>,
    signal: RwSignal<QueueState>,
) -> impl IntoView {
    // Progress changes one transfer. Memo equality prevents those updates
    // from rebuilding every completed row's DOM in a 200-result history.
    let current = Memo::new(move |_| signal.with(|s| s.rows.iter().find(|r| r.id == id).cloned()));
    let get = move || current.get();
    let initial = current.get_untracked().unwrap();
    View::new(
    xcss_web::html::tr()
        .child((
            xcss_web::html::td()
                .child(
                    #[allow(unused_braces)]
                    {
                        {
                            #[allow(unreachable_code)] #[allow(unused_mut)]
                            #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
                            component_view(
                                #[allow(clippy::needless_borrows_for_generic_args)]
                                &crate::workspace::Icon,
                                {
                                    let mut props = component_props_builder(
                                            &crate::workspace::Icon,
                                        )
                                        .name(#[allow(unused_braces)] { "file" })
                                        .build();
                                    props
                                },
                            )
                        }
                    },
                )
                .class("path cell-icon"),
            xcss_web::html::td()
                .child(
                    #[allow(unused_braces)]
                    {
                        xcss_web::html::a()
                            .child(
                                #[allow(unused_braces)]
                                {
                                    IntoRender::into_render({ initial.name })
                                },
                            )
                            .href(
                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                                    http::file_url(&initial.path),
                                ),
                            )
                    },
                )
                .class("path cell-name"),
            xcss_web::html::td()
                .child((
                    xcss_web::html::span()
                        .child(
                            #[allow(unused_braces)]
                            {
                                IntoRender::into_render({
                                    move || get().map(|r| r.message).unwrap_or_default()
                                })
                            },
                        )
                        .class((
                            "upload-failure",
                            move || {
                                get()
                                    .is_some_and(|r| {
                                        matches!(r.state.as_str(), "failed" | "unknown")
                                    })
                            },
                        ))
                        .class((
                            "upload-unknown",
                            move || get().is_some_and(|r| r.state == "unknown"),
                        )),
                    xcss_web::html::span()
                        .child(
                            #[allow(unused_braces)]
                            {
                                IntoRender::into_render({
                                    move || get().map(|r| r.speed).unwrap_or_default()
                                })
                            },
                        )
                        .class("upload-speed")
                        .aria_hidden("true"),
                    xcss_web::html::span()
                        .child(
                            #[allow(unused_braces)]
                            {
                                IntoRender::into_render({
                                    move || get().map(|r| r.progress).unwrap_or_default()
                                })
                            },
                        )
                        .class("upload-progress")
                        .aria_hidden("true"),
                    xcss_web::html::span()
                        .child(
                            #[allow(unused_braces)]
                            {
                                IntoRender::into_render({
                                    move || {
                                        get()
                                            .map(|r| {
                                                format!("{}: {} {}", r.name, r.message, r.progress)
                                            })
                                            .unwrap_or_default()
                                    }
                                })
                            },
                        )
                        .class("visually-hidden")
                        .role("status")
                        .aria_live("polite"),
                    {
                        #[allow(unreachable_code)] #[allow(unused_mut)]
                        #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
                        component_view(
                            #[allow(clippy::needless_borrows_for_generic_args)]
                            &Show,
                            {
                                let mut props = component_props_builder(
                                        &Show,
                                    )
                                    .when(
                                        #[allow(unused_braces)]
                                        {
                                            move || {
                                                get()
                                                    .is_some_and(|r| {
                                                        !r.terminal()
                                                            && !matches!(
                                                                r.phase.as_str(), "awaiting-confirmation" | "cleanup"
                                                            )
                                                    })
                                            }
                                        },
                                    )
                                    .children({
                                        ::xcss_web::children::ToChildren::to_children(move || {
                                            xcss_web::html::button()
                                                .child(
                                                    #[allow(unused_braces)]
                                                    {
                                                        IntoRender::into_render({
                                                            move || {
                                                                get()
                                                                    .map(|r| {
                                                                        if r.phase == "submitting" {
                                                                            t("停止等待", "Stop waiting")
                                                                        } else {
                                                                            t("取消", "Cancel")
                                                                        }
                                                                    })
                                                                    .unwrap_or_default()
                                                            }
                                                        })
                                                    },
                                                )
                                                .class("upload-cancel")
                                                .r#type("button")
                                                .aria_label(
                                                    ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                                    {
                                                        get()
                                                            .map(|r| {
                                                                format!(
                                                                    "{} {}", if r.state == "queued" { t("取消排队上传",
                                                                    "Cancel queued upload") } else if r.phase == "checking" {
                                                                    t("取消续传状态核对",
                                                                    "Cancel resume status check for") } else if r.phase ==
                                                                    "submitting" { t("停止等待上传",
                                                                    "Stop waiting for upload") } else { t("取消上传",
                                                                    "Cancel upload") }, r.name
                                                                )
                                                            })
                                                            .unwrap_or_default()
                                                    }),
                                                )
                                                .on(
                                                    xcss_web::ev::click,
                                                    move |_| cancel(model, id),
                                                )
                                        })
                                    })
                                    .build();
                                props
                            },
                        )
                    },
                    {
                        #[allow(unreachable_code)] #[allow(unused_mut)]
                        #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
                        component_view(
                            #[allow(clippy::needless_borrows_for_generic_args)]
                            &Show,
                            {
                                let mut props = component_props_builder(
                                        &Show,
                                    )
                                    .when(
                                        #[allow(unused_braces)]
                                        {
                                            move || {
                                                get()
                                                    .is_some_and(|r| r.terminal() && !r.recovery.is_empty())
                                            }
                                        },
                                    )
                                    .children({
                                        ::xcss_web::children::ToChildren::to_children(move || {
                                            xcss_web::html::button()
                                                .child(#[allow(unused_braces)] { "↻" })
                                                .class("retry-btn")
                                                .r#type("button")
                                                .attr(
                                                    "data-recovery",
                                                    ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                                    get().map(|r| r.recovery).unwrap_or_default()),
                                                )
                                                .aria_label(
                                                    ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                                    {
                                                        get()
                                                            .map(|r| {
                                                                format!(
                                                                    "{} {}", if r.recovery == "query_upload" {
                                                                    t("核对上传状态", "Check upload status") } else {
                                                                    t("重试上传", "Retry upload") }, r.name
                                                                )
                                                            })
                                                            .unwrap_or_default()
                                                    }),
                                                )
                                                .disabled(
                                                    ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                                    {
                                                        let _ = tick.get();
                                                        get().is_some_and(|r| js_sys::Date::now() < r.ready_at)
                                                    }),
                                                )
                                                .on(
                                                    xcss_web::ev::click,
                                                    move |_| recover(model, id),
                                                )
                                        })
                                    })
                                    .build();
                                props
                            },
                        )
                    },
                ))
                .class("cell-status upload-status")
                .id(
                    ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                        format!("uploadStatus{id}"),
                    ),
                )
                .aria_label(
                    ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                    get().map(|r| announce(&r)).unwrap_or_default()),
                ),
        ))
        .class("uploader")
        .id(
            ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                format!("upload{id}"),
            ),
        )
        .attr(
            "data-upload-id",
            ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(id),
        ),
)
}

fn announce(row: &Row) -> String {
    if row.state == "queued" && !row.recovery.is_empty() {
        return format!("{}: {}", row.name, t("等待重试", "waiting to retry"));
    }
    let message = match row.phase.as_str() {
        "completed" => t("上传完成", "upload complete"),
        "checking" => t("正在核对续传状态", "checking resume status"),
        "cleanup" => t("正在清理暂存上传", "cleaning up staged upload"),
        "submitting" => t(
            "上传数据已发送，等待服务器确认",
            "upload data sent; waiting for server confirmation",
        ),
        "awaiting-confirmation" => t("等待覆盖决定", "waiting for overwrite decision"),
        "unknown" => format!(
            "{}; {}",
            t("上传结果不确定", "upload result unknown"),
            row.message
        ),
        "cancelled" => {
            if row.message.contains("Skipped") {
                t(
                    "已跳过，因为目标已存在",
                    "skipped because the destination exists",
                )
            } else {
                t("上传已取消", "upload cancelled")
            }
        }
        _ => row.message.clone(),
    };
    format!("{}: {message}", row.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(id: u64, retired: u64, state: &str) -> Row {
        Row {
            id,
            batch: 0,
            name: String::new(),
            path: String::new(),
            size: 0,
            upload_id: String::new(),
            offset: 0,
            target_revision: None,
            state: state.into(),
            phase: state.into(),
            message: String::new(),
            progress: String::new(),
            speed: String::new(),
            recovery: String::new(),
            ready_at: 0.,
            visible: true,
            missing_retry: false,
            dispatched: false,
            retired,
            recovering_unknown: false,
        }
    }
    #[test]
    fn querying_unknown_rows_remains_unresolved_until_their_own_result_settles() {
        let mut first = item(0, 1, "running");
        first.recovering_unknown = true;
        let mut second = item(1, 2, "running");
        second.recovering_unknown = true;
        assert!(has_unresolved_unknown(&[first.clone(), second.clone()]));
        first.state = "completed".into();
        first.recovering_unknown = false;
        assert!(has_unresolved_unknown(&[first.clone(), second.clone()]));
        second.state = "failed".into();
        second.recovering_unknown = false;
        assert!(!has_unresolved_unknown(&[first, second]));
        let mut queue = QueueState::default();
        pause_auth_state(&mut queue);
        settle_pause(&mut queue, true);
        assert_eq!(queue.paused, "auth");
        settle_pause(&mut queue, false);
        assert_eq!(queue.paused, "auth");
    }
    #[test]
    fn evicting_an_unknown_history_row_does_not_resolve_its_uncertain_outcome() {
        let mut queue = QueueState {
            paused: "unknown".into(),
            unresolved: HashSet::from([0]),
            ..QueueState::default()
        };
        settle_pause(&mut queue, false);
        assert_eq!(queue.paused, "unknown");
        queue.unresolved.remove(&0);
        settle_pause(&mut queue, false);
        assert!(queue.paused.is_empty());
    }
    #[test]
    fn terminal_history_uses_completion_order_and_never_evicts_pending_rows() {
        let rows = vec![
            item(0, 30, "completed"),
            item(1, 10, "failed"),
            item(2, 0, "running"),
            item(3, 20, "cancelled"),
            item(4, 40, "unknown"),
        ];
        assert!(retired_history_ids(&rows, 4).is_empty());
        assert_eq!(retired_history_ids(&rows, 3), HashSet::from([1]));
        assert_eq!(retired_history_ids(&rows, 1), HashSet::from([0, 1, 3]));
        let pending = vec![item(7, 0, "queued"), item(8, 0, "running")];
        assert!(retired_history_ids(&pending, 0).is_empty());
        let rows = (0..201)
            .map(|id| item(id, id + 1, "completed"))
            .collect::<Vec<_>>();
        assert_eq!(retired_history_ids(&rows, 200), HashSet::from([0]));
    }
}
