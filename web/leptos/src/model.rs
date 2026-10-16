use crate::http::{self, Error};
use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Tag {
    pub id: u64,
    pub name: String,
    pub color: Option<String>,
    #[serde(default)]
    pub file_count: u64,
}
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub fn valid_tag(value: &serde_json::Value, count: bool) -> bool {
    value["id"]
        .as_u64()
        .is_some_and(|n| n > 0 && n <= MAX_SAFE_INTEGER)
        && value["name"].as_str().is_some_and(|s| s.len() <= 320)
        && match value.get("color") {
            Some(serde_json::Value::Null) => true,
            Some(serde_json::Value::String(c)) => {
                c.len() == 7 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit())
            }
            _ => false,
        }
        && (!count
            || value["file_count"]
                .as_u64()
                .is_some_and(|n| n <= MAX_SAFE_INTEGER))
}
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct File {
    pub path_type: String,
    pub name: String,
    pub mtime: u64,
    pub size: u64,
    pub revision: String,
    #[serde(default)]
    pub tags: Vec<Tag>,
    #[serde(default)]
    pub file_id: Option<u64>,
    #[serde(default)]
    pub tags_has_more: bool,
    #[serde(default = "yes")]
    pub tags_available: bool,
}
fn yes() -> bool {
    true
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filters {
    pub q: String,
    pub sort: String,
    pub order: String,
    pub all: String,
    pub any: String,
    pub exclude: String,
}
impl Filters {
    pub fn location() -> Self {
        let p = web_sys::UrlSearchParams::new_with_str(
            &http::window().location().search().unwrap_or_default(),
        )
        .unwrap();
        let sort = p
            .get("sort")
            .filter(|s| ["name", "mtime", "size"].contains(&s.as_str()))
            .unwrap_or_else(|| "name".into());
        let order = p
            .get("order")
            .filter(|s| ["asc", "desc"].contains(&s.as_str()))
            .unwrap_or_else(|| "asc".into());
        Self {
            q: p.get("q").unwrap_or_default(),
            sort,
            order,
            all: p.get("all").unwrap_or_default(),
            any: p.get("any").unwrap_or_default(),
            exclude: p.get("exclude").unwrap_or_default(),
        }
    }
    pub fn query(&self) -> String {
        http::query([
            ("q", self.q.as_str()),
            ("sort", self.sort.as_str()),
            ("order", self.order.as_str()),
            ("all", self.all.as_str()),
            ("any", self.any.as_str()),
            ("exclude", self.exclude.as_str()),
        ])
    }
}
#[derive(Clone, Debug, Default)]
pub struct Listing {
    pub files: Vec<File>,
    pub next: Option<String>,
    pub cursors: HashSet<String>,
    pub loading: bool,
    pub loaded: bool,
    pub invalidated: bool,
    pub retry: bool,
    pub status: String,
    pub window_start: usize,
    pub generation: u64,
}
#[derive(Clone, Debug)]
pub struct Rename {
    pub name: String,
    pub draft: String,
    pub error: String,
    pub pending: bool,
    pub created: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Dialog {
    pub open: bool,
    pub return_focus: String,
    pub title: String,
    pub message: String,
    pub label: String,
    pub value: String,
    pub confirm: String,
    pub alternate: String,
    pub cancel: String,
    pub danger: bool,
    pub color: String,
}
#[derive(Clone, Copy)]
pub struct Model {
    pub directory: RwSignal<String>,
    pub dir_exists: RwSignal<bool>,
    pub view: RwSignal<String>,
    pub filters: RwSignal<Filters>,
    pub listing: RwSignal<Listing>,
    pub action: RwSignal<String>,
    pub notice: RwSignal<String>,
    pub rename: RwSignal<Option<Rename>>,
    pub pending: RwSignal<HashSet<String>>,
    pub busy_actions: RwSignal<std::collections::HashMap<String, usize>>,
    pub operation_status: RwSignal<String>,
    pub dialog: RwSignal<Dialog>,
    pub tag_target: RwSignal<Option<File>>,
    pub tags_version: RwSignal<u64>,
}
impl Model {
    pub fn new(directory: String, exists: bool) -> Self {
        Self {
            directory: RwSignal::new(directory),
            dir_exists: RwSignal::new(exists),
            view: RwSignal::new(view()),
            filters: RwSignal::new(Filters::location()),
            listing: RwSignal::new(Listing::default()),
            action: RwSignal::new(String::new()),
            notice: RwSignal::new(String::new()),
            rename: RwSignal::new(None),
            pending: RwSignal::new(HashSet::new()),
            busy_actions: RwSignal::new(std::collections::HashMap::new()),
            operation_status: RwSignal::new(String::new()),
            dialog: RwSignal::new(Dialog::default()),
            tag_target: RwSignal::new(None),
            tags_version: RwSignal::new(0),
        }
    }
    pub fn path(self, name: &str) -> String {
        let dir = self.directory.get_untracked();
        format!("{}{name}", if dir == "/" { dir } else { format!("{dir}/") })
    }
    pub fn file(self, name: &str) -> Option<File> {
        self.listing
            .with_untracked(|l| l.files.iter().find(|f| f.name == name).cloned())
    }
    pub fn invalidate(self, unknown: bool) {
        LIST_REQUEST.with(|active| {
            if let Some(controller) = active.borrow_mut().take() {
                controller.abort();
            }
        });
        self.listing.update(|l| {
            l.generation += 1;
            l.loading = false;
            l.invalidated = true;
            l.retry = false;
            l.next = None;
            l.cursors.clear();
            l.status = if unknown {
                xcss_web::i18n::t(
                    "文件夹内容可能已变更，请刷新列表后再加载更多项目。",
                    "Folder contents may have changed; refresh the list before loading more items.",
                )
            } else {
                xcss_web::i18n::t(
                    "文件夹内容已变更，请刷新列表后再加载更多项目。",
                    "Folder contents changed; refresh the list before loading more items.",
                )
            };
        });
    }
    pub fn refresh_required(self) {
        self.invalidate(false);
        self.listing.update(|l| {
            l.status = xcss_web::i18n::t(
                "文件夹快照已过时，请刷新列表后再操作。",
                "The folder snapshot is stale; refresh the list before another operation.",
            )
        });
    }
    pub fn begin(self, key: &str, message: String) -> bool {
        if self.pending.with_untracked(|p| p.contains(key)) {
            return false;
        }
        self.pending.update(|p| {
            p.insert(key.to_owned());
        });
        self.operation_status.set(message);
        true
    }
    pub fn end(self, key: &str) {
        self.pending.update(|p| {
            p.remove(key);
        });
        if self.pending.with_untracked(|p| p.is_empty()) {
            self.operation_status.set(String::new());
        }
    }
    pub fn control_begin(self, mode: &str) {
        self.busy_actions
            .update(|m| *m.entry(mode.into()).or_default() += 1);
    }
    pub fn control_end(self, mode: &str) {
        self.busy_actions.update(|m| {
            if let Some(n) = m.get_mut(mode) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    m.remove(mode);
                }
            }
        });
    }
    pub fn tags_changed(self) {
        self.tags_version.update(|v| *v += 1);
    }
}
pub fn view() -> String {
    let h = http::window().location().hash().unwrap_or_default();
    match h.as_str() {
        "" | "#" | "#files" => "files",
        "#tags" => "tags",
        "#status" => "status",
        "#account" => "account",
        _ => "unknown",
    }
    .into()
}
pub fn parse_listing(value: serde_json::Value) -> Result<(Vec<File>, Option<String>), Error> {
    let invalid = || {
        Error::message(xcss_web::i18n::t(
            "文件列表响应无效",
            "Invalid file list response",
        ))
    };
    let paths = value["paths"]
        .as_array()
        .filter(|a| a.len() <= 200)
        .ok_or_else(invalid)?;
    let next = match value.get("next_cursor").ok_or_else(invalid)? {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) if !s.is_empty() && s.len() <= 1024 => Some(s.clone()),
        _ => return Err(invalid()),
    };
    let tags = match value.get("file_tags") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Array(tags)) if tags.len() == paths.len() => Some(tags),
        _ => return Err(invalid()),
    };
    let mut names = HashSet::new();
    let mut files = Vec::new();
    for (index, p) in paths.iter().enumerate() {
        let item_invalid = || {
            Error::message(xcss_web::i18n::t(
                "文件列表项目无效",
                "Invalid file list item",
            ))
        };
        let mut f: File = serde_json::from_value(p.clone()).map_err(|_| item_invalid())?;
        if !["Dir", "SymlinkDir", "File", "SymlinkFile"].contains(&f.path_type.as_str())
            || !http::valid_path(&f.name)
            || !crate::protocol::revision(&f.revision)
            || f.mtime > 9_007_199_254_740_991
            || f.size > 9_007_199_254_740_991
            || !names.insert(f.name.clone())
        {
            return Err(item_invalid());
        }
        f.tags_available = value.get("file_tags") != Some(&serde_json::Value::Null);
        if let Some(tags) = tags {
            let t = &tags[index];
            f.file_id = match t.get("file_id").ok_or_else(invalid)? {
                serde_json::Value::Null => None,
                v => Some(
                    v.as_u64()
                        .filter(|n| *n > 0 && *n <= MAX_SAFE_INTEGER)
                        .ok_or_else(invalid)?,
                ),
            };
            f.tags_has_more = t["tags_has_more"].as_bool().ok_or_else(invalid)?;
            if !t["tags"].as_array().is_some_and(|labels| {
                labels.len() <= 50 && labels.iter().all(|v| valid_tag(v, false))
            }) {
                return Err(invalid());
            }
            f.tags = serde_json::from_value(t["tags"].clone()).map_err(|_| invalid())?;
            if f.tags.len() > 50
                || f.tags.iter().any(|t| {
                    t.id == 0
                        || t.name.len() > 320
                        || t.color.as_ref().is_some_and(|c| {
                            c.len() != 7
                                || !c.starts_with('#')
                                || !c[1..].bytes().all(|b| b.is_ascii_hexdigit())
                        })
                })
            {
                return Err(invalid());
            }
        }
        files.push(f);
    }
    Ok((files, next))
}
thread_local! {static LIST_REQUEST:std::cell::RefCell<Option<web_sys::AbortController>>=const{std::cell::RefCell::new(None)};}
pub async fn load(model: Model, refresh: bool) {
    if model.listing.with_untracked(|l| l.loading) {
        if !refresh {
            return;
        }
        LIST_REQUEST.with(|c| {
            if let Some(c) = c.borrow_mut().take() {
                c.abort();
            }
        });
    }
    if !model.dir_exists.get_untracked() {
        model.listing.update(|l| {
            l.loaded = true;
            l.status.clear();
        });
        return;
    }
    if !refresh
        && model
            .listing
            .with_untracked(|l| l.loaded && l.next.is_none() && !l.invalidated && !l.retry)
    {
        return;
    }
    let refresh = refresh || model.listing.with_untracked(|l| l.invalidated);
    let old = model.listing.get_untracked();
    let focus_more = http::document()
        .active_element()
        .is_some_and(|e| e.class_list().contains("load-more"));
    let generation = old.generation + 1;
    let cursor = if refresh { None } else { old.next.clone() };
    model.listing.update(|l| {
        l.loading = true;
        l.retry = false;
        l.generation = generation;
        l.status = xcss_web::i18n::t("正在加载文件…", "Loading files…");
    });
    let params = model.filters.get_untracked();
    let mut q = params.query();
    q.push_str(&format!(
        "&path={}&limit=200",
        http::encode(&model.directory.get_untracked())
    ));
    if let Some(c) = &cursor {
        q.push_str(&format!("&cursor={}", http::encode(c)));
    }
    let controller = web_sys::AbortController::new().unwrap();
    LIST_REQUEST.with(|c| *c.borrow_mut() = Some(controller.clone()));
    let result = async {
        let mut attempt = 0;
        loop {
            match http::get(&format!("/__xczs__/api/list?{q}"), Some(controller.clone())).await {
                Ok(p) => break parse_listing(p),
                Err(e)
                    if attempt == 0
                        && cursor.is_none()
                        && e.trusted(409, "directory_changed")
                        && e.recovery == "refresh_target" =>
                {
                    attempt += 1
                }
                Err(e) => break Err(e),
            }
        }
    }
    .await;
    if model.listing.with_untracked(|l| l.generation) != generation {
        return;
    }
    LIST_REQUEST.with(|c| *c.borrow_mut() = None);
    match result {
        Ok((files, next)) => {
            let seen = if refresh {
                HashSet::new()
            } else {
                old.files.iter().map(|f| f.name.clone()).collect()
            };
            let repeated_cursor = next
                .as_ref()
                .is_some_and(|n| Some(n) == cursor.as_ref() || !refresh && old.cursors.contains(n));
            if files.iter().any(|f| seen.contains(&f.name)) || repeated_cursor {
                model.listing.update(|l| {
                    l.loading = false;
                    l.retry = true;
                    l.status = format!(
                        "{} {}",
                        xcss_web::i18n::t("无法加载文件列表：", "Unable to load the file list:"),
                        if repeated_cursor {
                            xcss_web::i18n::t(
                                "服务器重复返回了文件列表游标",
                                "The server repeated a file list cursor",
                            )
                        } else {
                            xcss_web::i18n::t(
                                "服务器重复返回了文件列表项目",
                                "The server repeated a file list item",
                            )
                        }
                    );
                });
                return;
            }
            // An inline editor owns its row until settled. A concurrent write
            // may refresh the listing, but must retain that row's identity,
            // draft, and DOM focus while replacing the surrounding snapshot.
            let pinned = model.rename.get_untracked().and_then(|editor| {
                old.files
                    .iter()
                    .find(|file| file.name == editor.name)
                    .cloned()
            });
            model.listing.update(|l| {
                if refresh {
                    l.files.clear();
                    l.cursors.clear();
                }
                let files = files
                    .into_iter()
                    .filter(|file| !refresh || pinned.as_ref().is_none_or(|p| p.name != file.name));
                l.files.extend(files);
                if refresh && let Some(pinned) = pinned {
                    l.files.insert(0, pinned);
                }
                l.next = next;
                if let Some(c) = &l.next {
                    l.cursors.insert(c.clone());
                }
                l.loading = false;
                l.loaded = true;
                l.invalidated = false;
                l.retry = false;
                l.window_start = l.files.len().saturating_sub(200);
                l.status = format!(
                    "{} {}",
                    l.files.len(),
                    xcss_web::i18n::t("项已加载", "items loaded")
                );
            });
        }
        Err(e) => model.listing.update(|l| {
            l.loading = false;
            l.retry = true;
            l.status = format!(
                "{} {}",
                xcss_web::i18n::t("无法加载文件列表：", "Unable to load the file list:"),
                e.message
            );
        }),
    }
    if focus_more
        && model
            .listing
            .with_untracked(|l| !l.retry && l.next.is_none())
    {
        gloo_timers::callback::Timeout::new(0, || {
            use wasm_bindgen::JsCast;
            if let Some(el) = http::document()
                .query_selector(".list-status")
                .ok()
                .flatten()
                .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
            {
                let _ = el.focus();
            }
        })
        .forget();
    }
}
pub fn size(value: u64) -> String {
    if value < 1024 {
        return format!("{value} B");
    }
    let mut n = value as f64;
    let mut index: usize = 0;
    let units = ["KiB", "MiB", "GiB", "TiB"];
    while n >= 1024. && index < 4 {
        n /= 1024.;
        index += 1;
    }
    format!("{n:.1} {}", units[index.saturating_sub(1)])
}
pub fn mtime(value: u64) -> String {
    if value == 0 {
        return String::new();
    }
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(value as f64));
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
        d.get_utc_full_year(),
        d.get_utc_month() + 1,
        d.get_utc_date(),
        d.get_utc_hours(),
        d.get_utc_minutes()
    )
}

pub fn listing_status(l: &Listing) -> String {
    if l.loading || l.invalidated || l.retry || !l.loaded || l.files.is_empty() {
        return l.status.clone();
    }
    let mut status = if l.next.is_none() {
        format!(
            "{} {} {}",
            xcss_web::i18n::t("全部", "All"),
            l.files.len(),
            xcss_web::i18n::t("项已加载", "items loaded")
        )
    } else {
        format!(
            "{} {}",
            l.files.len(),
            xcss_web::i18n::t("项已加载", "items loaded")
        )
    };
    if l.window_start > 0 || l.files.len() > 200 {
        status.push_str(&format!(
            "; {} {}–{} ({} {}) {}",
            xcss_web::i18n::t("显示第", "showing items"),
            l.window_start + 1,
            (l.window_start + 200).min(l.files.len()),
            l.files.len().min(200),
            xcss_web::i18n::t("项可见", "visible"),
            xcss_web::i18n::t("当前窗口", "in this window")
        ));
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn a_page_is_atomic_and_revisions_are_required() {
        let valid = json!({"paths":[{"path_type":"File","name":"a","mtime":0,"size":0,"revision":"a".repeat(64)}],"next_cursor":null});
        assert!(parse_listing(valid.clone()).is_ok());
        let mut invalid = valid;
        invalid["paths"].as_array_mut().unwrap().push(
            json!({"path_type":"File","name":"../b","mtime":0,"size":0,"revision":"a".repeat(64)}),
        );
        assert!(parse_listing(invalid).is_err());
    }
}
