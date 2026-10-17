use crate::{
    http::{self, Error},
    model::{Model, Tag},
};
use leptos::component::{component_props_builder, component_view};
use leptos::prelude::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::AbortController;
use xcss_web::i18n::t;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct TagPage {
    pub tags: Vec<Tag>,
    pub previous_cursor: Option<String>,
    pub next_cursor: Option<String>,
}
#[derive(Clone, Debug, Default, Deserialize)]
pub struct FolderPage {
    pub folders: Vec<String>,
    pub previous_cursor: Option<String>,
    pub next_cursor: Option<String>,
}
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Status {
    pub root: String,
    pub available: bool,
    pub last_scan_at: Option<u64>,
    pub indexed: u64,
    pub missing: u64,
    pub suspect: u64,
    pub last_error: Option<String>,
}
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Listed {
    pub file_id: Option<u64>,
    pub tags: Vec<Tag>,
    pub tags_has_more: bool,
}
thread_local! {static TAG_QUEUE:std::rc::Rc<futures::lock::Mutex<()>>=std::rc::Rc::new(futures::lock::Mutex::new(()));}
pub async fn request(path: &str, method: &str, body: Option<Value>) -> Result<Value, Error> {
    request_inner(path, method, body, None).await
}
fn cancelled(controller: &Option<AbortController>) -> bool {
    controller.as_ref().is_some_and(|c| c.signal().aborted())
}
async fn retry_delay(milliseconds: u32, controller: &Option<AbortController>) -> Result<(), Error> {
    if cancelled(controller) {
        return Err(Error::network(false));
    }
    let Some(controller) = controller else {
        gloo_timers::future::TimeoutFuture::new(milliseconds).await;
        return Ok(());
    };
    let signal = controller.signal();
    let (sender, receiver) = futures::channel::oneshot::channel::<()>();
    let mut sender = Some(sender);
    let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        if let Some(sender) = sender.take() {
            let _ = sender.send(());
        }
    });
    let _ = signal.add_event_listener_with_callback("abort", listener.as_ref().unchecked_ref());
    // An obsolete read must relinquish the serialized SQLite queue even while
    // waiting on Retry-After. Dropping the losing timer cancels that delay.
    let _ = futures::future::select(
        Box::pin(gloo_timers::future::TimeoutFuture::new(milliseconds)),
        Box::pin(receiver),
    )
    .await;
    let _ = signal.remove_event_listener_with_callback("abort", listener.as_ref().unchecked_ref());
    if signal.aborted() {
        Err(Error::network(false))
    } else {
        Ok(())
    }
}
async fn request_inner(
    path: &str,
    method: &str,
    body: Option<Value>,
    controller: Option<AbortController>,
) -> Result<Value, Error> {
    // SQLite reads share the same admission queue as writes. Mutations are
    // dispatched once; only explicit temporary GET failures may be repeated.
    let queue = TAG_QUEUE.with(std::rc::Rc::clone);
    let _guard = queue.lock().await;
    for attempt in 0..3 {
        if cancelled(&controller) {
            return Err(Error::network(false));
        }
        let mut headers = vec![(http::CSRF.into(), http::token())];
        if body.is_some() {
            headers.push(("Content-Type".into(), "application/json".into()));
        }
        let result = http::raw_bounded(
            &format!("/api/v1/file-tags{path}"),
            method,
            headers,
            body.as_ref()
                .map(|b| wasm_bindgen::JsValue::from_str(&b.to_string())),
            controller.clone(),
            http::ResponseBounds {
                timeout: if path == "/scan" { 90_000 } else { 5_000 },
                unknown: method != "GET",
                success_limit: 8 * 1024 * 1024,
            },
        )
        .await;
        let r = match result {
            Ok(response) => response,
            Err(error) => {
                if error.status == 401 {
                    let _ = http::window().location().replace("/__xczs__/login");
                }
                return Err(error);
            }
        };
        if r.ok() {
            return if r.status == 204 || r.body.is_empty() {
                Ok(Value::Null)
            } else {
                r.json()
                    .and_then(|value| validate_mutation(path, method, value))
            };
        }
        let e = r.upload_error(false);
        if r.status == 401 {
            let _ = http::window().location().replace("/__xczs__/login");
        }
        if method == "GET"
            && e.retryable
            && attempt < 2
            && matches!(
                (e.status, e.code.as_str()),
                (429, "too_many_requests") | (503, "service_unavailable")
            )
            && e.retry_after.unwrap_or(1) <= 2
        {
            retry_delay(e.retry_after.unwrap_or(1) as u32 * 1000, &controller).await?;
            continue;
        }
        return Err(e);
    }
    unreachable!()
}
fn valid_cursor(c: &Option<String>) -> bool {
    c.as_ref().is_none_or(|c| {
        !c.is_empty()
            && c.len() <= 4096
            && c.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    })
}
pub fn parse_tags(v: Value) -> Result<TagPage, Error> {
    if v.get("previous_cursor").is_none() || v.get("next_cursor").is_none() {
        return Err(Error::message(t("标签响应无效", "Invalid tag response")));
    }
    if !v["tags"].as_array().is_some_and(|tags| {
        tags.len() <= 50 && tags.iter().all(|v| crate::model::valid_tag(v, true))
    }) {
        return Err(Error::message(t("标签响应无效", "Invalid tag response")));
    }
    let p: TagPage = serde_json::from_value(v)
        .map_err(|_| Error::message(t("标签响应无效", "Invalid tag response")))?;
    let mut ids = HashSet::new();
    if p.tags.len() > 50
        || !valid_cursor(&p.previous_cursor)
        || !valid_cursor(&p.next_cursor)
        || p.tags.iter().any(|t| {
            (t.id == 0 || t.id > 9_007_199_254_740_991 || t.file_count > 9_007_199_254_740_991)
                || t.name.len() > 320
                || !ids.insert(t.id)
                || t.color.as_ref().is_some_and(|c| {
                    c.len() != 7
                        || !c.starts_with('#')
                        || !c[1..].bytes().all(|b| b.is_ascii_hexdigit())
                })
        })
    {
        return Err(Error::message(t("标签响应无效", "Invalid tag response")));
    }
    Ok(p)
}
fn parse_folders(value: Value) -> Result<FolderPage, Error> {
    let invalid = || Error::message(t("目录响应无效", "Invalid folder response"));
    if value.get("previous_cursor").is_none() || value.get("next_cursor").is_none() {
        return Err(invalid());
    }
    let page: FolderPage = serde_json::from_value(value).map_err(|_| invalid())?;
    let mut names = HashSet::new();
    if !valid_cursor(&page.previous_cursor)
        || !valid_cursor(&page.next_cursor)
        || page.folders.len() > 50
        || page.folders.iter().any(|name| {
            name.is_empty() || name.len() > 768 || name.contains('/') || !names.insert(name)
        })
    {
        return Err(invalid());
    }
    Ok(page)
}
fn parse_status(value: Value) -> Result<Status, Error> {
    let invalid = || Error::message(t("状态响应无效", "Invalid status response"));
    if value.get("last_scan_at").is_none() || value.get("last_error").is_none() {
        return Err(invalid());
    }
    let status: Status = serde_json::from_value(value).map_err(|_| invalid())?;
    if status.root.len() > 4096
        || [status.indexed, status.missing, status.suspect]
            .iter()
            .any(|n| *n > crate::model::MAX_SAFE_INTEGER)
        || status
            .last_scan_at
            .is_some_and(|n| n > crate::model::MAX_SAFE_INTEGER)
    {
        return Err(invalid());
    }
    Ok(status)
}
fn parse_listed(value: Value) -> Result<Listed, Error> {
    let invalid = || Error::message(t("文件标签响应无效", "Invalid file tag response"));
    if value.get("file_id").is_none()
        || !value["tags"].as_array().is_some_and(|tags| {
            tags.len() <= 50 && tags.iter().all(|v| crate::model::valid_tag(v, false))
        })
    {
        return Err(invalid());
    }
    let listed: Listed = serde_json::from_value(value).map_err(|_| invalid())?;
    if listed
        .file_id
        .is_some_and(|n| n == 0 || n > crate::model::MAX_SAFE_INTEGER)
    {
        return Err(invalid());
    }
    Ok(listed)
}
fn validate_mutation(path: &str, method: &str, value: Value) -> Result<Value, Error> {
    let valid = match (path, method) {
        ("/scan", "POST") => value["scanned"]
            .as_u64()
            .is_some_and(|n| n <= crate::model::MAX_SAFE_INTEGER),
        ("/backup", "POST") => value["filename"]
            .as_str()
            .is_some_and(|s| !s.is_empty() && s.len() <= 255),
        ("/tags", "POST") => value["id"]
            .as_u64()
            .is_some_and(|n| n > 0 && n <= crate::model::MAX_SAFE_INTEGER),
        _ => true,
    };
    if valid {
        Ok(value)
    } else {
        Err(Error::message(t(
            "服务器响应无效",
            "Invalid server response",
        )))
    }
}
async fn page_read(
    cursor: Option<String>,
    controller: Option<AbortController>,
) -> Result<TagPage, Error> {
    parse_tags(
        request_inner(
            &format!(
                "/tags{}",
                cursor
                    .map(|c| format!("?cursor={}", http::encode(&c)))
                    .unwrap_or_default()
            ),
            "GET",
            None,
            controller,
        )
        .await?,
    )
}
fn failure(e: &Error) -> String {
    match e.code.as_str() {
        "tag_limit_exceeded" => t(
            "每个文件最多添加 16 个标签，请先移除现有标签。",
            "Each file can have at most 16 tags. Remove an existing tag first.",
        ),
        "capacity_exhausted" => t(
            "已达到持久容量限制，请先整理数据或备份文件。",
            "Storage capacity reached. Organize your data or backups before retrying.",
        ),
        _ => t(
            "无法读取或更新标签，请稍后重试。",
            "Could not read or update tags. Please try again.",
        ),
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Selections {
    all: Vec<String>,
    any: Vec<String>,
    exclude: Vec<String>,
}
fn filter_ids(value: &str) -> Vec<String> {
    value
        .split(',')
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 19
                && !id.starts_with('0')
                && id.bytes().all(|b| b.is_ascii_digit())
        })
        .take(100)
        .map(str::to_owned)
        .collect()
}
impl Selections {
    fn filters(filters: &crate::model::Filters) -> Self {
        Self {
            all: filter_ids(&filters.all),
            any: filter_ids(&filters.any),
            exclude: filter_ids(&filters.exclude),
        }
    }
    fn group(&self, key: &str) -> &Vec<String> {
        match key {
            "all" => &self.all,
            "any" => &self.any,
            _ => &self.exclude,
        }
    }
    fn group_mut(&mut self, key: &str) -> &mut Vec<String> {
        match key {
            "all" => &mut self.all,
            "any" => &mut self.any,
            _ => &mut self.exclude,
        }
    }
    fn contains(&self, id: u64) -> bool {
        let id = id.to_string();
        [&self.all, &self.any, &self.exclude]
            .iter()
            .any(|group| group.contains(&id))
    }
}
fn merge_candidates(previous: Vec<Tag>, incoming: &[Tag], selected: &Selections) -> Vec<Tag> {
    let page: HashSet<_> = incoming.iter().map(|tag| tag.id).collect();
    let mut result = previous
        .into_iter()
        .filter(|tag| page.contains(&tag.id) || selected.contains(tag.id))
        .collect::<Vec<_>>();
    for tag in incoming {
        if let Some(index) = result.iter().position(|old| old.id == tag.id) {
            result[index] = tag.clone();
        } else {
            result.push(tag.clone());
        }
    }
    result
}
fn filter_options(candidates: &[Tag], selected: &[String]) -> Vec<(String, String)> {
    selected
        .iter()
        .filter(|id| !candidates.iter().any(|tag| tag.id.to_string() == **id))
        .map(|id| (id.clone(), format!("#{id}")))
        .chain(
            candidates
                .iter()
                .map(|tag| (tag.id.to_string(), tag.name.clone())),
        )
        .collect()
}

#[component]
pub fn TagFilters(model: Model) -> impl IntoView {
    let p = RwSignal::new(TagPage::default());
    let candidates = RwSignal::new(Vec::<Tag>::new());
    let draft = RwSignal::new(Selections::filters(&model.filters.get_untracked()));
    let error = RwSignal::new(String::new());
    let cursor = RwSignal::new(None::<String>);
    let open = RwSignal::new(false);
    let generation = StoredValue::new(0u64);
    let last_version = StoredValue::new(model.tags_version.get_untracked());
    let active = StoredValue::new_local(None::<AbortController>);
    on_cleanup(move || {
        if let Some(controller) = active.get_value() {
            controller.abort();
        }
    });
    Effect::new(move |_| {
        draft.set(Selections::filters(&model.filters.get()));
    });
    Effect::new(move |_| {
        let version = model.tags_version.get();
        let next = cursor.get();
        let opened = open.get();
        let view = model.view.get();
        if let Some(controller) = active.get_value() {
            controller.abort();
        }
        generation.update_value(|n| *n += 1);
        let request_id = generation.get_value();
        if last_version.get_value() != version {
            last_version.set_value(version);
            candidates.set(vec![]);
            p.set(TagPage::default());
            if next.is_some() {
                cursor.set(None);
                return;
            }
        }
        if !opened || view != "files" {
            return;
        }
        let controller = AbortController::new().unwrap();
        active.set_value(Some(controller.clone()));
        leptos::task::spawn_local(async move {
            let result = page_read(next, Some(controller.clone())).await;
            if controller.signal().aborted() || generation.get_value() != request_id {
                return;
            }
            match result {
                Ok(v) => {
                    candidates.update(|current| {
                        *current = merge_candidates(
                            std::mem::take(current),
                            &v.tags,
                            &draft.get_untracked(),
                        )
                    });
                    p.set(v);
                    error.set(String::new());
                }
                Err(e) => error.set(failure(&e)),
            }
        });
    });
    View::new(
        leptos::html::details()
            .child((
                leptos::html::summary()
                    .child(
                        #[allow(unused_braces)]
                        {
                            IntoRender::into_render({ t("标签", "Tags") })
                        },
                    )
                    .aria_label(::leptos::prelude::IntoAttributeValue::into_attribute_value(
                        t("标签筛选", "Tag filters"),
                    )),
                leptos::html::div()
                    .child((
                        leptos::html::p()
                            .child(
                                #[allow(unused_braces)]
                                {
                                    IntoRender::into_render({ move || error.get() })
                                },
                            )
                            .role("alert")
                            .hidden(::leptos::prelude::IntoAttributeValue::into_attribute_value(
                                move || error.get().is_empty(),
                            )),
                        IntoRender::into_render({
                            [
                                ("all", t("全部标签", "All tags")),
                                ("any", t("任一标签", "Any tag")),
                                ("exclude", t("排除标签", "Exclude tags")),
                            ]
                            .into_iter()
                            .map(move |(key, label)| {
                                View::new(
    leptos::html::label()
        .child((
            IntoRender::into_render(label.clone()),
            leptos::html::select()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            move || {
                                let selected = draft.get().group(key).clone();
                                let tags = candidates.get();
                                let options = filter_options(&tags, &selected);
                                options
                                    .into_iter()
                                    .map(move |(id, name)| {
                                        let chosen = selected.contains(&id);
                                        View::new(
    leptos::html::option()
        .child(
            #[allow(unused_braces)]
            { IntoRender::into_render({ name }) },
        )
        .value(::leptos::prelude::IntoAttributeValue::into_attribute_value(id))
        .selected(
            ::leptos::prelude::IntoAttributeValue::into_attribute_value(chosen),
        ),
)
                                    })
                                    .collect_view()
                            }
                        })
                    },
                )
                .class("xcss-select")
                .name(
                    ::leptos::prelude::IntoAttributeValue::into_attribute_value(key),
                )
                .multiple(true)
                .size("3")
                .aria_label(
                    ::leptos::prelude::IntoAttributeValue::into_attribute_value(
                        label,
                    ),
                )
                .on(
                    leptos::ev::change,
                    move |event| {
                        let select = event_target::<
                            web_sys::HtmlSelectElement,
                        >(&event);
                        if let Ok(options) = select
                            .query_selector_all("option:checked")
                        {
                            let values = (0..options.length())
                                .filter_map(|index| options.item(index))
                                .filter_map(|option| {
                                    option.dyn_into::<web_sys::HtmlOptionElement>().ok()
                                })
                                .take(100)
                                .map(|option| option.value())
                                .collect();
                            draft
                                .update(|selection| *selection.group_mut(key) = values);
                        }
                    },
                ),
        ))
        .class("xcss-form-field"),
)
                            })
                            .collect_view()
                        }),
                        leptos::html::div()
                            .child((
                                leptos::html::button()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({
                                                t("上一页", "Previous page")
                                            })
                                        },
                                    )
                                    .class("xcss-button")
                                    .r#type("button")
                                    .disabled(
                                        ::leptos::prelude::IntoAttributeValue::into_attribute_value(
                                            move || p.get().previous_cursor.is_none(),
                                        ),
                                    )
                                    .on(leptos::ev::click, move |_| {
                                        cursor.set(p.get_untracked().previous_cursor)
                                    }),
                                leptos::html::button()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({ t("下一页", "Next page") })
                                        },
                                    )
                                    .class("xcss-button")
                                    .r#type("button")
                                    .disabled(
                                        ::leptos::prelude::IntoAttributeValue::into_attribute_value(
                                            move || p.get().next_cursor.is_none(),
                                        ),
                                    )
                                    .on(leptos::ev::click, move |_| {
                                        cursor.set(p.get_untracked().next_cursor)
                                    }),
                            ))
                            .class("search-tag-pager"),
                        leptos::html::div()
                            .child((
                                leptos::html::button()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({ t("搜索", "Search") })
                                        },
                                    )
                                    .class("xcss-button")
                                    .r#type("submit")
                                    .on(leptos::ev::click, move |_| open.set(false)),
                                leptos::html::button()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({
                                                t("清除标签筛选", "Clear tag filters")
                                            })
                                        },
                                    )
                                    .class("xcss-button")
                                    .r#type("button")
                                    .on(leptos::ev::click, move |_| {
                                        open.set(false);
                                        model.filters.update(|f| {
                                            f.all.clear();
                                            f.any.clear();
                                            f.exclude.clear();
                                        });
                                        crate::workspace::apply_filters(model);
                                    }),
                            ))
                            .class("search-tag-actions"),
                    ))
                    .class("search-tag-options"),
            ))
            .class("search-tags")
            .open(::leptos::prelude::IntoAttributeValue::into_attribute_value(
                move || open.get(),
            ))
            .on(leptos::ev::toggle, move |e| {
                let target = event_target::<web_sys::HtmlElement>(&e);
                open.set(target.has_attribute("open"));
            }),
    )
}

#[component]
pub fn FileTagToolbar(model: Model) -> impl IntoView {
    let file = RwSignal::new(Listed::default());
    let assigned = RwSignal::new(TagPage::default());
    let choices = RwSignal::new(TagPage::default());
    let error = RwSignal::new(String::new());
    let reading = RwSignal::new(false);
    let pending = RwSignal::new(false);
    let busy = Memo::new(move |_| reading.get() || pending.get());
    let last_target = StoredValue::new(None::<String>);
    let added = RwSignal::new(Vec::<Tag>::new());
    let assigned_cursor = RwSignal::new(None::<String>);
    let choices_cursor = RwSignal::new(None::<String>);
    let version = RwSignal::new(0u64);
    let generation = RwSignal::new(0u64);
    let active = StoredValue::new_local(None::<AbortController>);
    on_cleanup(move || {
        if let Some(controller) = active.get_value() {
            controller.abort();
        }
    });
    Effect::new(move |_| {
        let target = model.tag_target.get();
        let _ = version.get();
        let _ = model.tags_version.get();
        let cursor = assigned_cursor.get();
        let choice_cursor = choices_cursor.get();
        generation.update(|n| *n += 1);
        let request_id = generation.get_untracked();
        if let Some(controller) = active.get_value() {
            controller.abort();
        }
        let identity = target.as_ref().map(|file| file.name.clone());
        if last_target.get_value() != identity {
            last_target.set_value(identity);
            added.set(vec![]);
            pending.set(false);
            assigned_cursor.set(None);
            choices_cursor.set(None);
            if cursor.is_some() || choice_cursor.is_some() {
                return;
            }
        }
        if model.action.get() != "tags" {
            reading.set(false);
            return;
        }
        let Some(target) = target else {
            reading.set(false);
            return;
        };
        let controller = AbortController::new().unwrap();
        active.set_value(Some(controller.clone()));
        file.set(Listed::default());
        assigned.set(TagPage::default());
        choices.set(TagPage::default());
        error.set(String::new());
        reading.set(true);
        leptos::task::spawn_local(async move {
            let result = async {
                let current = parse_listed(
                    request_inner(
                        &format!(
                            "/file?path={}",
                            http::encode(model.path(&target.name).trim_start_matches('/'))
                        ),
                        "GET",
                        None,
                        Some(controller.clone()),
                    )
                    .await?,
                )?;
                let mut tags = TagPage::default();
                let mut available = TagPage::default();
                if let Some(id) = current.file_id {
                    tags = parse_tags(
                        request_inner(
                            &format!(
                                "/files/{id}/tags{}",
                                cursor
                                    .map(|c| format!("?cursor={}", http::encode(&c)))
                                    .unwrap_or_default()
                            ),
                            "GET",
                            None,
                            Some(controller.clone()),
                        )
                        .await?,
                    )?;
                    available = page_read(choice_cursor, Some(controller.clone())).await?;
                }
                Ok((current, tags, available))
            }
            .await;
            if controller.signal().aborted() || generation.get_untracked() != request_id {
                return;
            }
            match result {
                Ok((current, tags, available)) => {
                    if !current.tags_has_more {
                        added.update(|added| {
                            added.retain(|tag| {
                                current.tags.iter().any(|current| current.id == tag.id)
                            })
                        });
                    }
                    file.set(current);
                    assigned.set(tags);
                    choices.set(available);
                    error.set(String::new());
                }
                Err(e) => error.set(failure(&e)),
            }
            reading.set(false);
        });
    });
    let mutate = move |id: u64, action: &'static str| {
        if busy.get_untracked() {
            return;
        }
        let Some(file_id) = file.get_untracked().file_id else {
            return;
        };
        pending.set(true);
        let selected_tag = assigned
            .with_untracked(|page| page.tags.iter().find(|tag| tag.id == id).cloned())
            .or_else(|| {
                choices.with_untracked(|page| page.tags.iter().find(|tag| tag.id == id).cloned())
            })
            .or_else(|| added.with_untracked(|tags| tags.iter().find(|tag| tag.id == id).cloned()));
        let original = model.tag_target.get_untracked().map(|f| f.name);
        leptos::task::spawn_local(async move {
            match request(
                "/file-tags",
                "POST",
                Some(json!({"file_ids":[file_id],"tag_ids":[id],"action":action})),
            )
            .await
            {
                Ok(_) => {
                    if original == model.tag_target.get_untracked().map(|f| f.name) {
                        added.update(|tags| {
                            tags.retain(|tag| tag.id != id);
                            if action == "add"
                                && let Some(tag) = selected_tag
                            {
                                tags.push(tag);
                            }
                        });
                        assigned_cursor.set(None);
                        reading.set(true);
                        version.update(|v| *v += 1);
                    }
                    model.tags_changed();
                    model.invalidate(false);
                }
                Err(e) => {
                    if original == model.tag_target.get_untracked().map(|file| file.name) {
                        error.set(failure(&e));
                    }
                }
            }
            if original == model.tag_target.get_untracked().map(|f| f.name) {
                pending.set(false);
            }
        });
    };
    View::new(
    leptos::html::section()
        .child((
            leptos::html::span()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            move || {
                                model.tag_target.get().map(|f| f.name).unwrap_or_default()
                            }
                        })
                    },
                )
                .class("file-tag-target"),
            leptos::html::div()
                .child((
                    leptos::html::p()
                        .child((
                            IntoRender::into_render({
                                move || error.get()
                            }),
                            leptos::html::button()
                                .child(
                                    #[allow(unused_braces)]
                                    {
                                        IntoRender::into_render({
                                            t("重试", "Retry")
                                        })
                                    },
                                )
                                .class("xcss-button")
                                .r#type("button")
                                .on(
                                    leptos::ev::click,
                                    move |_| version.update(|v| *v += 1),
                                ),
                        ))
                        .role("alert")
                        .hidden(
                            ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                            error.get().is_empty()),
                        ),
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
                                    .when(#[allow(unused_braces)] { move || busy.get() })
                                    .children({
                                        ::leptos::children::ToChildren::to_children(move || {
                                            leptos::html::p()
                                                .child(
                                                    #[allow(unused_braces)]
                                                    {
                                                        IntoRender::into_render({
                                                            t("加载中…", "Loading…")
                                                        })
                                                    },
                                                )
                                                .role("status")
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
                                                !reading.get() && error.get().is_empty()
                                                    && file.get().file_id.is_none()
                                            }
                                        },
                                    )
                                    .children({
                                        ::leptos::children::ToChildren::to_children(move || {
                                            leptos::html::div()
                                                .child((
                                                    leptos::html::p()
                                                        .child(
                                                            #[allow(unused_braces)]
                                                            {
                                                                IntoRender::into_render({
                                                                    t(
                                                                        "该文件尚未索引。添加标签前需要扫描整个文件库，这可能需要较长时间。",
                                                                        "This file is not indexed. Scan the entire file library before adding tags; this may take a while.",
                                                                    )
                                                                })
                                                            },
                                                        ),
                                                    leptos::html::button()
                                                        .child(
                                                            #[allow(unused_braces)]
                                                            {
                                                                IntoRender::into_render({
                                                                    t("扫描整个文件库", "Scan entire library")
                                                                })
                                                            },
                                                        )
                                                        .class("xcss-button")
                                                        .disabled(
                                                            ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                                            busy.get()),
                                                        )
                                                        .on(
                                                            leptos::ev::click,
                                                            move |_| {
                                                                if busy.get_untracked() {
                                                                    return;
                                                                }
                                                                pending.set(true);
                                                                let original = model
                                                                    .tag_target
                                                                    .get_untracked()
                                                                    .map(|file| file.name);
                                                                leptos::task::spawn_local(async move {
                                                                    let result = request("/scan", "POST", None).await;
                                                                    let current = original
                                                                        == model.tag_target.get_untracked().map(|file| file.name);
                                                                    match result {
                                                                        Ok(_) => {
                                                                            if current {
                                                                                reading.set(true);
                                                                                version.update(|v| *v += 1);
                                                                            }
                                                                            model.tags_changed();
                                                                        }
                                                                        Err(_) => {
                                                                            if current {
                                                                                error
                                                                                    .set(
                                                                                        t(
                                                                                            "扫描结果未能确认，请查看文件库状态后再决定是否重试。",
                                                                                            "The scan result could not be confirmed. Check Library status before retrying.",
                                                                                        ),
                                                                                    );
                                                                            }
                                                                        }
                                                                    }
                                                                    if current {
                                                                        pending.set(false);
                                                                    }
                                                                });
                                                            },
                                                        ),
                                                    leptos::html::a()
                                                        .child(
                                                            #[allow(unused_braces)]
                                                            {
                                                                IntoRender::into_render({
                                                                    t("查看文件库状态", "Library status")
                                                                })
                                                            },
                                                        )
                                                        .href("#status"),
                                                ))
                                                .role("status")
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
                                        { move || file.get().file_id.is_some() },
                                    )
                                    .children({
                                        ::leptos::children::ToChildren::to_children(move || (
                                            leptos::html::span()
                                                .child(
                                                    #[allow(unused_braces)]
                                                    {
                                                        IntoRender::into_render({
                                                            move || {
                                                                let current = file.get();
                                                                if current.tags_has_more {
                                                                    t(
                                                                        "标签超过 16 个，请先移除至少于 16 个。",
                                                                        "More than 16 tags. Remove tags until fewer than 16 remain.",
                                                                    )
                                                                } else {
                                                                    format!(
                                                                        "{} {}/16", t("标签：", "Tags:"), current.tags.len()
                                                                    )
                                                                }
                                                            }
                                                        })
                                                    },
                                                )
                                                .class("file-tag-count"),
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
                                                                        file
                                                                            .with(|current| {
                                                                                !current.tags_has_more && current.tags.len() >= 16
                                                                            })
                                                                    }
                                                                },
                                                            )
                                                            .children({
                                                                ::leptos::children::ToChildren::to_children(move || {
                                                                    leptos::html::span()
                                                                        .child(
                                                                            #[allow(unused_braces)]
                                                                            {
                                                                                IntoRender::into_render({
                                                                                    t(
                                                                                        "每个文件最多 16 个标签，请先移除后再添加。",
                                                                                        "Each file can have at most 16 tags. Remove a tag before adding another.",
                                                                                    )
                                                                                })
                                                                            },
                                                                        )
                                                                })
                                                            })
                                                            .build();
                                                        props
                                                    },
                                                )
                                            },
                                            leptos::html::div()
                                                .child((
                                                    IntoRender::into_render({
                                                        move || {
                                                            let current = file.get();
                                                            let mut selected = current.tags.clone();
                                                            for tag in assigned
                                                                .get()
                                                                .tags
                                                                .into_iter()
                                                                .chain(added.get())
                                                            {
                                                                if !selected.iter().any(|value| value.id == tag.id) {
                                                                    selected.push(tag);
                                                                }
                                                            }
                                                            let ids: HashSet<_> = selected
                                                                .iter()
                                                                .map(|tag| tag.id)
                                                                .collect();
                                                            let mut tags = if current.tags_has_more
                                                                || current.tags.len() > 16
                                                            {
                                                                assigned.get().tags
                                                            } else {
                                                                selected
                                                            };
                                                            if !current.tags_has_more && current.tags.len() <= 16 {
                                                                for tag in choices.get().tags {
                                                                    if !tags.iter().any(|t| t.id == tag.id) {
                                                                        tags.push(tag);
                                                                    }
                                                                }
                                                            }
                                                            tags.into_iter()
                                                                .map(move |tag| {
                                                                    let selected = ids.contains(&tag.id);
                                                                    let label = format!(
                                                                        "{} {}", if selected { t("移除标签", "Remove tag") }
                                                                        else { t("添加标签", "Add tag") }, tag.name
                                                                    );
                                                                    let disabled = current.tags_has_more
                                                                        || current.tags.len() >= 16;
                                                                    View::new(
    leptos::html::button()
        .child(
            #[allow(unused_braces)]
            {
                leptos::html::span()
                    .child(
                        #[allow(unused_braces)]
                        { IntoRender::into_render({ tag.name }) },
                    )
                    .class("file-tag-label")
                    .style((
                        "border-inline-start-color",
                        tag.color.unwrap_or_default(),
                    ))
            },
        )
        .class("file-tag-choice xcss-button")
        .r#type("button")
        .aria_pressed(
            ::leptos::prelude::IntoAttributeValue::into_attribute_value(
                selected.to_string(),
            ),
        )
        .aria_label(
            ::leptos::prelude::IntoAttributeValue::into_attribute_value(label),
        )
        .disabled(
            ::leptos::prelude::IntoAttributeValue::into_attribute_value(move || {
                busy.get() || disabled && !selected
            }),
        )
        .on(
            leptos::ev::click,
            move |_| mutate(tag.id, if selected { "remove" } else { "add" }),
        ),
)
                                                                })
                                                                .collect_view()
                                                        }
                                                    }),
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
                                                                                !busy.get() && assigned.with(|page| page.tags.is_empty())
                                                                                    && choices.with(|page| page.tags.is_empty())
                                                                            }
                                                                        },
                                                                    )
                                                                    .children({
                                                                        ::leptos::children::ToChildren::to_children(move || {
                                                                            leptos::html::span()
                                                                                .child(
                                                                                    #[allow(unused_braces)]
                                                                                    {
                                                                                        IntoRender::into_render({
                                                                                            t(
                                                                                                "请先在标签管理中创建标签。",
                                                                                                "Create tags in Manage tags first.",
                                                                                            )
                                                                                        })
                                                                                    },
                                                                                )
                                                                        })
                                                                    })
                                                                    .build();
                                                                props
                                                            },
                                                        )
                                                    },
                                                ))
                                                .class("file-tag-options")
                                                .role("group")
                                                .aria_label(
                                                    ::leptos::prelude::IntoAttributeValue::into_attribute_value(
                                                        t("选择标签", "Select tags"),
                                                    ),
                                                ),
                                            leptos::html::div()
                                                .child((
                                                    leptos::html::button()
                                                        .child(
                                                            #[allow(unused_braces)]
                                                            {
                                                                IntoRender::into_render({
                                                                    t(
                                                                        "已添加标签：上一页",
                                                                        "Assigned tags: previous page",
                                                                    )
                                                                })
                                                            },
                                                        )
                                                        .class("xcss-button")
                                                        .disabled(
                                                            ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                                            busy.get() || assigned.get().previous_cursor.is_none()),
                                                        )
                                                        .on(
                                                            leptos::ev::click,
                                                            move |_| {
                                                                assigned_cursor
                                                                    .set(assigned.get_untracked().previous_cursor)
                                                            },
                                                        ),
                                                    leptos::html::button()
                                                        .child(
                                                            #[allow(unused_braces)]
                                                            {
                                                                IntoRender::into_render({
                                                                    t("已添加标签：下一页", "Assigned tags: next page")
                                                                })
                                                            },
                                                        )
                                                        .class("xcss-button")
                                                        .disabled(
                                                            ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                                            busy.get() || assigned.get().next_cursor.is_none()),
                                                        )
                                                        .on(
                                                            leptos::ev::click,
                                                            move |_| {
                                                                assigned_cursor.set(assigned.get_untracked().next_cursor)
                                                            },
                                                        ),
                                                    leptos::html::button()
                                                        .child(
                                                            #[allow(unused_braces)]
                                                            {
                                                                IntoRender::into_render({
                                                                    t(
                                                                        "可用标签：上一页",
                                                                        "Available tags: previous page",
                                                                    )
                                                                })
                                                            },
                                                        )
                                                        .class("xcss-button")
                                                        .disabled(
                                                            ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                                            {
                                                                busy.get() || file.get().tags_has_more
                                                                    || choices.get().previous_cursor.is_none()
                                                            }),
                                                        )
                                                        .on(
                                                            leptos::ev::click,
                                                            move |_| {
                                                                choices_cursor.set(choices.get_untracked().previous_cursor)
                                                            },
                                                        ),
                                                    leptos::html::button()
                                                        .child(
                                                            #[allow(unused_braces)]
                                                            {
                                                                IntoRender::into_render({
                                                                    t("可用标签：下一页", "Available tags: next page")
                                                                })
                                                            },
                                                        )
                                                        .class("xcss-button")
                                                        .disabled(
                                                            ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                                            {
                                                                busy.get() || file.get().tags_has_more
                                                                    || choices.get().next_cursor.is_none()
                                                            }),
                                                        )
                                                        .on(
                                                            leptos::ev::click,
                                                            move |_| {
                                                                choices_cursor.set(choices.get_untracked().next_cursor)
                                                            },
                                                        ),
                                                ))
                                                .class("search-tag-pager"),
                                        ))
                                    })
                                    .build();
                                props
                            },
                        )
                    },
                ))
                .class("file-tag-editor"),
        ))
        .class("xcss-content-panel file-tag-bar")
        .aria_label(
            ::leptos::prelude::IntoAttributeValue::into_attribute_value(
                t("文件标签", "File tags"),
            ),
        )
        .hidden(
            ::leptos::prelude::IntoAttributeValue::into_attribute_value(move || {
                model.action.get() != "tags" || model.tag_target.get().is_none()
            }),
        ),
)
}

#[component]
pub fn Library(model: Model) -> impl IntoView {
    let tags = RwSignal::new(TagPage::default());
    let cursor = RwSignal::new(None::<String>);
    let status = RwSignal::new(None::<Status>);
    let folders = RwSignal::new(FolderPage::default());
    let folder_path = RwSignal::new(model.directory.get_untracked().trim_matches('/').to_owned());
    let folder_cursor = RwSignal::new(None::<String>);
    let version = RwSignal::new(0u64);
    let error = RwSignal::new(String::new());
    let notice = RwSignal::new(String::new());
    let pending = RwSignal::new(false);
    let new_tag = RwSignal::new(String::new());
    let new_color = RwSignal::new("#3b82f6".to_owned());
    let editor = RwSignal::new(None::<(Tag, bool)>);
    let editor_name = RwSignal::new(String::new());
    let editor_color = RwSignal::new("#3b82f6".to_owned());
    let editor_error = RwSignal::new(String::new());
    let close_editor = Callback::new(move |()| {
        if !pending.get_untracked() {
            editor.set(None);
        }
    });
    let open_editor = move |tag: Tag, deleting: bool| {
        editor_name.set(tag.name.clone());
        editor_color.set(tag.color.clone().unwrap_or_else(|| "#3b82f6".into()));
        editor_error.set(String::new());
        error.set(String::new());
        editor.set(Some((tag, deleting)));
    };
    let save_editor = Callback::new(move |()| {
        if pending.get_untracked() {
            return;
        }
        let Some((tag, deleting)) = editor.get_untracked() else {
            return;
        };
        let body = (!deleting).then(
            || json!({"name":editor_name.get_untracked(),"color":editor_color.get_untracked()}),
        );
        pending.set(true);
        editor_error.set(String::new());
        leptos::task::spawn_local(async move {
            let result = request(
                &format!("/tags/{}", tag.id),
                if deleting { "DELETE" } else { "PUT" },
                body,
            )
            .await;
            let current = editor
                .try_get_untracked()
                .flatten()
                .is_some_and(|(active, action)| active.id == tag.id && action == deleting);
            match result {
                Ok(_) => {
                    if current {
                        editor.set(None);
                    }
                    notice.set(if deleting {
                        t("标签已删除。", "Tag deleted.")
                    } else {
                        t("标签已更新。", "Tag updated.")
                    });
                    model.tags_changed();
                    cursor.set(None);
                    version.update(|v| *v += 1);
                }
                Err(e) => {
                    if current {
                        editor_error.set(failure(&e));
                    }
                }
            }
            let _ = pending.try_set(false);
        });
    });
    let generation = StoredValue::new(0u64);
    let active = StoredValue::new_local(None::<AbortController>);
    on_cleanup(move || {
        if let Some(controller) = active.get_value() {
            controller.abort();
        }
    });
    Effect::new(move |_| {
        let view = model.view.get();
        let _ = version.get();
        let _ = model.tags_version.get();
        let cursor = cursor.get();
        let path = folder_path.get();
        let fc = folder_cursor.get();
        generation.update_value(|n| *n += 1);
        let request_id = generation.get_value();
        if let Some(controller) = active.get_value() {
            controller.abort();
        }
        if view != "tags" {
            editor.set(None);
            editor_error.set(String::new());
        }
        if !matches!(view.as_str(), "tags" | "status") {
            return;
        }
        let controller = AbortController::new().unwrap();
        active.set_value(Some(controller.clone()));
        error.set(String::new());
        leptos::task::spawn_local(async move {
            let live = || !controller.signal().aborted() && generation.get_value() == request_id;
            if view == "tags" {
                match page_read(cursor, Some(controller.clone())).await {
                    Ok(p) if live() => tags.set(p),
                    Err(e) if live() => error.set(failure(&e)),
                    _ => return,
                }
                let q = http::query([("path", path), ("cursor", fc.unwrap_or_default())]);
                match request_inner(
                    &format!("/folders?{q}"),
                    "GET",
                    None,
                    Some(controller.clone()),
                )
                .await
                .and_then(parse_folders)
                {
                    Ok(p) if live() => folders.set(p),
                    Err(e) if live() => error.set(failure(&e)),
                    _ => {}
                }
            } else {
                match request_inner("/status", "GET", None, Some(controller.clone()))
                    .await
                    .and_then(parse_status)
                {
                    Ok(p) if live() => status.set(Some(p)),
                    Err(e) if live() => error.set(failure(&e)),
                    _ => {}
                }
            }
        });
    });
    let perform = move |path: String, method: &'static str, body: Option<Value>| {
        if pending.get_untracked() {
            return;
        }
        pending.set(true);
        error.set(String::new());
        let submitted_name = if path == "/tags" {
            body.as_ref()
                .and_then(|value| value["name"].as_str())
                .map(str::to_owned)
        } else {
            None
        };
        leptos::task::spawn_local(async move {
            match request(&path,method,body).await{Ok(v)=>{let text=if path=="/backup"{format!("{} {}",t("备份已保存：","Backup saved:"),v["filename"].as_str().unwrap_or_default())}else if path=="/scan"{format!("{} {}",t("扫描完成，读取文件数：","Scan complete. Files read:"),v["scanned"])}else{t("操作完成。","Operation completed.")};notice.set(text);model.tags_changed();cursor.set(None);if submitted_name.as_ref().is_some_and(|name|*name==new_tag.get_untracked()){new_tag.set(String::new());}},Err(e)=>error.set(if path=="/scan"{t("扫描结果未能确认，请检查上次扫描时间和错误信息后再决定是否重试。","The scan result could not be confirmed. Check the last scan time and errors before retrying.")}else{failure(&e)})}
            pending.set(false);
            version.update(|v| *v += 1);
        });
    };
    View::new(
    leptos::html::div()
        .child((
            leptos::html::p()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            move || notice.get()
                        })
                    },
                )
                .class("library-notice")
                .role("status")
                .hidden(
                    ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                    notice.get().is_empty()),
                ),
            leptos::html::p()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            move || error.get()
                        })
                    },
                )
                .role("alert")
                .hidden(
                    ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                    error.get().is_empty()),
                ),
            leptos::html::div()
                .child((
                    leptos::html::section()
                        .child((
                            leptos::html::h2()
                                .child(
                                    #[allow(unused_braces)]
                                    {
                                        IntoRender::into_render({
                                            t("索引目录", "Indexed folders")
                                        })
                                    },
                                ),
                            leptos::html::p()
                                .child(
                                    #[allow(unused_braces)]
                                    {
                                        IntoRender::into_render({
                                            t(
                                                "浏览已索引文件所在的目录。空目录不在索引中。",
                                                "Browse folders containing indexed files. Empty folders are not indexed.",
                                            )
                                        })
                                    },
                                ),
                            leptos::html::p()
                                .child(
                                    #[allow(unused_braces)]
                                    {
                                        leptos::html::a()
                                            .child(
                                                #[allow(unused_braces)]
                                                {
                                                    IntoRender::into_render({
                                                        move || {
                                                            let p = folder_path.get();
                                                            if p.is_empty() { "/".to_owned() } else { p }
                                                        }
                                                    })
                                                },
                                            )
                                            .href(
                                                ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                                {
                                                    format!("/{}/", http::file_url(& folder_path.get()))
                                                        .replace("//", "/")
                                                }),
                                            )
                                    },
                                ),
                            leptos::html::button()
                                .child(
                                    #[allow(unused_braces)]
                                    {
                                        IntoRender::into_render({
                                            t("上级目录", "Parent folder")
                                        })
                                    },
                                )
                                .class("xcss-button")
                                .hidden(
                                    ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                    folder_path.get().is_empty()),
                                )
                                .on(
                                    leptos::ev::click,
                                    move |_| {
                                        folder_cursor.set(None);
                                        folder_path
                                            .update(|p| {
                                                *p = p
                                                    .rsplit_once('/')
                                                    .map(|(p, _)| p.to_owned())
                                                    .unwrap_or_default();
                                            });
                                    },
                                ),
                            leptos::html::ul()
                                .child(
                                    #[allow(unused_braces)]
                                    {
                                        IntoRender::into_render({
                                            move || {
                                                folders
                                                    .get()
                                                    .folders
                                                    .into_iter()
                                                    .map(move |name| {
                                                        let path = folder_path.get();
                                                        let child = if path.is_empty() {
                                                            name.clone()
                                                        } else {
                                                            format!("{path}/{name}")
                                                        };
                                                        let href = format!("/{}/", http::file_url(& child));
                                                        View::new(
    leptos::html::li()
        .child((
            leptos::html::button()
                .child(
                    #[allow(unused_braces)]
                    { IntoRender::into_render({ name }) },
                )
                .class("xcss-button")
                .on(
                    leptos::ev::click,
                    move |_| {
                        folder_cursor.set(None);
                        folder_path.set(child.clone());
                    },
                ),
            leptos::html::a()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            t("打开文件目录", "Open folder")
                        })
                    },
                )
                .href(
                    ::leptos::prelude::IntoAttributeValue::into_attribute_value(href),
                ),
        )),
)
                                                    })
                                                    .collect_view()
                                            }
                                        })
                                    },
                                ),
                            leptos::html::nav()
                                .child((
                                    leptos::html::button()
                                        .child(
                                            #[allow(unused_braces)]
                                            {
                                                IntoRender::into_render({
                                                    t("首页", "First page")
                                                })
                                            },
                                        )
                                        .class("xcss-button")
                                        .disabled(
                                            ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                            folders.get().previous_cursor.is_none()),
                                        )
                                        .on(
                                            leptos::ev::click,
                                            move |_| folder_cursor.set(None),
                                        ),
                                    leptos::html::button()
                                        .child(
                                            #[allow(unused_braces)]
                                            {
                                                IntoRender::into_render({
                                                    t("上一页", "Previous page")
                                                })
                                            },
                                        )
                                        .class("xcss-button")
                                        .disabled(
                                            ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                            folders.get().previous_cursor.is_none()),
                                        )
                                        .on(
                                            leptos::ev::click,
                                            move |_| {
                                                folder_cursor.set(folders.get_untracked().previous_cursor)
                                            },
                                        ),
                                    leptos::html::button()
                                        .child(
                                            #[allow(unused_braces)]
                                            {
                                                IntoRender::into_render({
                                                    t("下一页", "Next page")
                                                })
                                            },
                                        )
                                        .class("xcss-button")
                                        .disabled(
                                            ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                            folders.get().next_cursor.is_none()),
                                        )
                                        .on(
                                            leptos::ev::click,
                                            move |_| {
                                                folder_cursor.set(folders.get_untracked().next_cursor)
                                            },
                                        ),
                                ))
                                .class("library-pager")
                                .aria_label(
                                    ::leptos::prelude::IntoAttributeValue::into_attribute_value(
                                        t("索引目录分页", "Indexed folder pages"),
                                    ),
                                ),
                        ))
                        .class("xcss-content-panel library-panel")
                        .aria_label(
                            ::leptos::prelude::IntoAttributeValue::into_attribute_value(
                                t("索引目录", "Indexed folders"),
                            ),
                        ),
                    leptos::html::form()
                        .child((
                            leptos::html::label()
                                .child((
                                    IntoRender::into_render({
                                        t("标签名称", "Tag name")
                                    }),
                                    leptos::html::input()
                                        .class("xcss-input")
                                        .required(true)
                                        .maxlength("80")
                                        .prop("value", move || new_tag.get())
                                        .on(
                                            leptos::ev::input,
                                            move |e| new_tag.set(event_target_value(&e)),
                                        ),
                                ))
                                .class("xcss-form-field"),
                            leptos::html::label()
                                .child((
                                    IntoRender::into_render({
                                        t("颜色", "Color")
                                    }),
                                    leptos::html::input()
                                        .class("xcss-input")
                                        .r#type("color")
                                        .prop("value", move || new_color.get())
                                        .on(
                                            leptos::ev::input,
                                            move |e| new_color.set(event_target_value(&e)),
                                        ),
                                ))
                                .class("xcss-form-field"),
                            leptos::html::button()
                                .child(
                                    #[allow(unused_braces)]
                                    {
                                        IntoRender::into_render({
                                            t("创建标签", "Create tag")
                                        })
                                    },
                                )
                                .class("xcss-button")
                                .r#type("submit")
                                .disabled(
                                    ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                    pending.get()),
                                ),
                        ))
                        .class("xcss-content-panel library-panel library-tag-form")
                        .on(
                            leptos::ev::submit,
                            move |e| {
                                e.prevent_default();
                                perform(
                                    "/tags".into(),
                                    "POST",
                                    Some(
                                        json!(
                                            { "name" : new_tag.get_untracked(), "color" : new_color
                                            .get_untracked() }
                                        ),
                                    ),
                                );
                            },
                        ),
                    leptos::html::section()
                        .child((
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
                                                { move || tags.get().tags.is_empty() },
                                            )
                                            .children({
                                                ::leptos::children::ToChildren::to_children(move || {
                                                    leptos::html::p()
                                                        .child(
                                                            #[allow(unused_braces)]
                                                            {
                                                                IntoRender::into_render({
                                                                    t("尚未创建标签。", "No tags created yet.")
                                                                })
                                                            },
                                                        )
                                                })
                                            })
                                            .build();
                                        props
                                    },
                                )
                            },
                            leptos::html::table()
                                .child((
                                    leptos::html::thead()
                                        .child(
                                            #[allow(unused_braces)]
                                            {
                                                leptos::html::tr()
                                                    .child((
                                                        leptos::html::th()
                                                            .child(
                                                                #[allow(unused_braces)]
                                                                {
                                                                    IntoRender::into_render({
                                                                        t("标签", "Tags")
                                                                    })
                                                                },
                                                            ),
                                                        leptos::html::th()
                                                            .child(
                                                                #[allow(unused_braces)]
                                                                {
                                                                    IntoRender::into_render({
                                                                        t("文件数", "File count")
                                                                    })
                                                                },
                                                            ),
                                                        leptos::html::th()
                                                            .child(
                                                                #[allow(unused_braces)]
                                                                {
                                                                    IntoRender::into_render({
                                                                        t("操作", "Actions")
                                                                    })
                                                                },
                                                            ),
                                                    ))
                                            },
                                        ),
                                    leptos::html::tbody()
                                        .child(
                                            #[allow(unused_braces)]
                                            {
                                                IntoRender::into_render({
                                                    move || {
                                                        tags
                                                            .get()
                                                            .tags
                                                            .into_iter()
                                                            .map(move |tag| {
                                                                let delete_tag = tag.clone();
                                                                let edit_tag = tag.clone();
                                                                View::new(
    leptos::html::tr()
        .child((
            leptos::html::td()
                .child((
                    leptos::html::span()
                        .class("library-color")
                        .style((
                            "background-color",
                            tag.color.unwrap_or_else(|| "#3b82f6".into()),
                        )),
                    IntoRender::into_render({ tag.name }),
                )),
            leptos::html::td()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            tag.file_count
                        })
                    },
                ),
            leptos::html::td()
                .child(
                    #[allow(unused_braces)]
                    {
                        leptos::html::div()
                            .child((
                                leptos::html::button()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({
                                                t("编辑", "Edit")
                                            })
                                        },
                                    )
                                    .class("xcss-button")
                                    .disabled(
                                        ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                        pending.get()),
                                    )
                                    .on(
                                        leptos::ev::click,
                                        move |_| open_editor(edit_tag.clone(), false),
                                    ),
                                leptos::html::button()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({
                                                t("删除整个标签", "Delete tag")
                                            })
                                        },
                                    )
                                    .class("xcss-button xcss-danger")
                                    .disabled(
                                        ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                        pending.get()),
                                    )
                                    .on(
                                        leptos::ev::click,
                                        move |_| open_editor(delete_tag.clone(), true),
                                    ),
                            ))
                            .class("library-row-actions")
                    },
                ),
        )),
)
                                                            })
                                                            .collect_view()
                                                    }
                                                })
                                            },
                                        ),
                                ))
                                .class("xcss-table")
                                .aria_label(
                                    ::leptos::prelude::IntoAttributeValue::into_attribute_value(
                                        t("标签列表", "Tag list"),
                                    ),
                                ),
                            leptos::html::nav()
                                .child((
                                    leptos::html::button()
                                        .child(
                                            #[allow(unused_braces)]
                                            {
                                                IntoRender::into_render({
                                                    t("首页", "First page")
                                                })
                                            },
                                        )
                                        .class("xcss-button")
                                        .disabled(
                                            ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                            tags.get().previous_cursor.is_none()),
                                        )
                                        .on(
                                            leptos::ev::click,
                                            move |_| cursor.set(None),
                                        ),
                                    leptos::html::button()
                                        .child(
                                            #[allow(unused_braces)]
                                            {
                                                IntoRender::into_render({
                                                    t("上一页", "Previous page")
                                                })
                                            },
                                        )
                                        .class("xcss-button")
                                        .disabled(
                                            ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                            tags.get().previous_cursor.is_none()),
                                        )
                                        .on(
                                            leptos::ev::click,
                                            move |_| cursor.set(tags.get_untracked().previous_cursor),
                                        ),
                                    leptos::html::button()
                                        .child(
                                            #[allow(unused_braces)]
                                            {
                                                IntoRender::into_render({
                                                    t("下一页", "Next page")
                                                })
                                            },
                                        )
                                        .class("xcss-button")
                                        .disabled(
                                            ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                            tags.get().next_cursor.is_none()),
                                        )
                                        .on(
                                            leptos::ev::click,
                                            move |_| cursor.set(tags.get_untracked().next_cursor),
                                        ),
                                ))
                                .class("library-pager")
                                .aria_label(
                                    ::leptos::prelude::IntoAttributeValue::into_attribute_value(
                                        t("标签列表分页", "Tag list pages"),
                                    ),
                                ),
                        ))
                        .class("xcss-content-panel library-panel"),
                ))
                .hidden(
                    ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                    model.view.get() != "tags"),
                ),
            IntoRender::into_render({
                move || {
                    editor
                        .get()
                        .map(|(tag, deleting)| {
                            View::new({
    #[allow(unreachable_code)] #[allow(unused_mut)] #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
    component_view(
        #[allow(clippy::needless_borrows_for_generic_args)]
        &xcss_web::components::Dialog,
        {
            let mut props = component_props_builder(
                    &xcss_web::components::Dialog,
                )
                .title(
                    #[allow(unused_braces)]
                    {
                        if deleting {
                            t("删除整个标签", "Delete tag")
                        } else {
                            t("编辑标签", "Edit tag")
                        }
                    },
                )
                .close(#[allow(unused_braces)] { close_editor })
                .children({
                    ::leptos::children::ToChildren::to_children(move || {
                        leptos::html::form()
                            .child((
                                IntoRender::into_render({
                                    deleting
                                        .then(|| {
                                            View::new((
    leptos::html::p()
        .child(
            #[allow(unused_braces)]
            {
                IntoRender::into_render({
                    if xcss_web::i18n::locale() == "en" {
                        format!(
                            "Deleting “{}” removes its associations with {} files. Original files are retained.",
                            tag.name, tag.file_count
                        )
                    } else {
                        format!(
                            "删除“{}”将移除它与 {} 个文件的关联，原文件不会被删除。",
                            tag.name, tag.file_count
                        )
                    }
                })
            },
        ),
    leptos::html::p()
        .child(
            #[allow(unused_braces)]
            {
                IntoRender::into_render({
                    t(
                        "请确认删除整个标签。",
                        "Confirm deletion of this tag.",
                    )
                })
            },
        ),
))
                                        })
                                }),
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
                                                .when(#[allow(unused_braces)] { move || !deleting })
                                                .children({
                                                    ::leptos::children::ToChildren::to_children(move || (
                                                        leptos::html::label()
                                                            .child((
                                                                IntoRender::into_render({
                                                                    t("标签名称", "Tag name")
                                                                }),
                                                                leptos::html::input()
                                                                    .class("xcss-input")
                                                                    .required(true)
                                                                    .maxlength("80")
                                                                    .attr("data-xcss-initial-focus", "")
                                                                    .readonly(
                                                                        ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                                                        pending.get()),
                                                                    )
                                                                    .prop("value", move || editor_name.get())
                                                                    .on(
                                                                        leptos::ev::input,
                                                                        move |e| editor_name.set(event_target_value(&e)),
                                                                    ),
                                                            ))
                                                            .class("xcss-form-field"),
                                                        leptos::html::label()
                                                            .child((
                                                                IntoRender::into_render({
                                                                    t("颜色", "Color")
                                                                }),
                                                                leptos::html::input()
                                                                    .class("xcss-input")
                                                                    .r#type("color")
                                                                    .disabled(
                                                                        ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                                                        pending.get()),
                                                                    )
                                                                    .prop("value", move || editor_color.get())
                                                                    .on(
                                                                        leptos::ev::input,
                                                                        move |e| editor_color.set(event_target_value(&e)),
                                                                    ),
                                                            ))
                                                            .class("xcss-form-field"),
                                                    ))
                                                })
                                                .build();
                                            props
                                        },
                                    )
                                },
                                leptos::html::p()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({
                                                move || editor_error.get()
                                            })
                                        },
                                    )
                                    .role("alert")
                                    .hidden(
                                        ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                        editor_error.get().is_empty()),
                                    ),
                                leptos::html::div()
                                    .child((
                                        leptos::html::button()
                                            .child(
                                                #[allow(unused_braces)]
                                                {
                                                    IntoRender::into_render({
                                                        t("取消", "Cancel")
                                                    })
                                                },
                                            )
                                            .class("xcss-button")
                                            .r#type("button")
                                            .attr(
                                                "data-xcss-initial-focus",
                                                ::leptos::prelude::IntoAttributeValue::into_attribute_value(
                                                    deleting.then_some(""),
                                                ),
                                            )
                                            .disabled(
                                                ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                                pending.get()),
                                            )
                                            .on(
                                                leptos::ev::click,
                                                move |_| close_editor.run(()),
                                            ),
                                        leptos::html::button()
                                            .child(
                                                #[allow(unused_braces)]
                                                {
                                                    IntoRender::into_render({
                                                        if deleting {
                                                            t("确认", "Confirm")
                                                        } else {
                                                            t("保存", "Save")
                                                        }
                                                    })
                                                },
                                            )
                                            .class("xcss-button")
                                            .class(("xcss-danger", deleting))
                                            .r#type("submit")
                                            .disabled(
                                                ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                                pending.get()),
                                            ),
                                    ))
                                    .class("library-actions"),
                            ))
                            .class("library-dialog-form")
                            .aria_busy(
                                ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                pending.get().to_string()),
                            )
                            .on(
                                leptos::ev::submit,
                                move |event| {
                                    event.prevent_default();
                                    save_editor.run(());
                                },
                            )
                    })
                })
                .build();
            props
        },
    )
})
                        })
                }
            }),
            leptos::html::section()
                .child((
                    leptos::html::p()
                        .child(
                            #[allow(unused_braces)]
                            {
                                IntoRender::into_render({
                                    t(
                                        "重新扫描会更新整个文件库的索引。扫描期间请等待完成；若结果未能确认，请先检查扫描状态。",
                                        "Scanning updates the entire file library index. Wait for completion; if the result is unconfirmed, check scan status before retrying.",
                                    )
                                })
                            },
                        ),
                    IntoRender::into_render({
                        move || {
                            status
                                .get()
                                .map(|s| {
                                    View::new(
    leptos::html::dl()
        .child((
            leptos::html::dt()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            t("原文件根目录", "Original file root")
                        })
                    },
                ),
            leptos::html::dd()
                .child(
                    #[allow(unused_braces)]
                    { IntoRender::into_render({ s.root }) },
                ),
            leptos::html::dt()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            t("目录状态", "Directory status")
                        })
                    },
                ),
            leptos::html::dd()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            if s.available {
                                t("可用", "Available")
                            } else {
                                t("异常", "Unavailable")
                            }
                        })
                    },
                ),
            leptos::html::dt()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            t("上次完整扫描", "Last full scan")
                        })
                    },
                ),
            leptos::html::dd()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            s.last_scan_at
                                .map(|v| crate::model::mtime(v * 1000))
                                .unwrap_or_else(|| t("尚未完成", "Not completed yet"))
                        })
                    },
                ),
            leptos::html::dt()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            t("当前文件", "Present files")
                        })
                    },
                ),
            leptos::html::dd()
                .child(
                    #[allow(unused_braces)]
                    { IntoRender::into_render({ s.indexed }) },
                ),
            leptos::html::dt()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            t("缺失记录", "Missing records")
                        })
                    },
                ),
            leptos::html::dd()
                .child(
                    #[allow(unused_braces)]
                    { IntoRender::into_render({ s.missing }) },
                ),
            leptos::html::dt()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            t("待确认记录", "Unconfirmed records")
                        })
                    },
                ),
            leptos::html::dd()
                .child(
                    #[allow(unused_braces)]
                    { IntoRender::into_render({ s.suspect }) },
                ),
            leptos::html::dt()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            t("最近扫描错误", "Last scan error")
                        })
                    },
                ),
            leptos::html::dd()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            if s.last_error.is_some() {
                                t(
                                    "扫描未能完成，请重新扫描。",
                                    "The scan could not complete. Scan again.",
                                )
                            } else {
                                t("无", "None")
                            }
                        })
                    },
                ),
        ))
        .class("library-status"),
)
                                })
                        }
                    }),
                    leptos::html::div()
                        .child((
                            leptos::html::button()
                                .child(
                                    #[allow(unused_braces)]
                                    {
                                        IntoRender::into_render({
                                            t("刷新扫描状态", "Refresh scan status")
                                        })
                                    },
                                )
                                .class("xcss-button")
                                .disabled(
                                    ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                    pending.get()),
                                )
                                .on(
                                    leptos::ev::click,
                                    move |_| version.update(|v| *v += 1),
                                ),
                            leptos::html::button()
                                .child(
                                    #[allow(unused_braces)]
                                    {
                                        IntoRender::into_render({
                                            move || {
                                                if pending.get() {
                                                    t("操作进行中…", "Operation in progress…")
                                                } else {
                                                    t("重新扫描", "Scan again")
                                                }
                                            }
                                        })
                                    },
                                )
                                .class("xcss-button")
                                .disabled(
                                    ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                    pending.get()),
                                )
                                .on(
                                    leptos::ev::click,
                                    move |_| perform("/scan".into(), "POST", None),
                                ),
                            leptos::html::button()
                                .child(
                                    #[allow(unused_braces)]
                                    {
                                        IntoRender::into_render({
                                            t("备份标签数据库", "Back up tag database")
                                        })
                                    },
                                )
                                .class("xcss-button")
                                .disabled(
                                    ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                                    pending.get()),
                                )
                                .on(
                                    leptos::ev::click,
                                    move |_| perform("/backup".into(), "POST", None),
                                ),
                        ))
                        .class("library-actions"),
                ))
                .class("xcss-content-panel library-panel")
                .hidden(
                    ::leptos::prelude::IntoAttributeValue::into_attribute_value(move ||
                    model.view.get() != "status"),
                ),
        ))
        .class("library-page")
        .hidden(
            ::leptos::prelude::IntoAttributeValue::into_attribute_value(move || {
                !matches!(model.view.get().as_str(), "tags" | "status")
            }),
        ),
)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filter_drafts_survive_candidate_pages_and_deleted_ids_remain_removable() {
        let tag = |id, name: &str| Tag {
            id,
            name: name.into(),
            color: None,
            file_count: 0,
        };
        let selected = Selections {
            all: vec!["1".into()],
            any: vec!["3".into()],
            exclude: vec!["7".into()],
        };
        let first = vec![
            tag(1, "Selected"),
            tag(2, "Unselected"),
            tag(3, "Selected elsewhere"),
        ];
        let second = merge_candidates(first, &[tag(4, "Next page")], &selected);
        assert_eq!(
            second.iter().map(|tag| tag.id).collect::<Vec<_>>(),
            vec![1, 3, 4]
        );
        let refreshed = merge_candidates(vec![], &[tag(2, "Remaining")], &selected);
        assert_eq!(
            filter_options(&refreshed, &selected.all),
            vec![("1".into(), "#1".into()), ("2".into(), "Remaining".into())]
        );
        assert_eq!(
            filter_options(&refreshed, &[]),
            vec![("2".into(), "Remaining".into())]
        );
        let renamed = merge_candidates(second, &[tag(1, "Renamed")], &selected);
        assert_eq!(
            renamed.iter().find(|tag| tag.id == 1).unwrap().name,
            "Renamed"
        );
        assert_eq!(selected.all, vec!["1"]);
    }
    #[test]
    fn filter_url_ids_reject_invalid_values_and_bound_each_group() {
        assert_eq!(
            filter_ids("1,0,01,-1,2.3,9999999999999999999,10000000000000000000,2"),
            vec!["1", "9999999999999999999", "2"]
        );
        let many = (1..=101)
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(filter_ids(&many).len(), 100);
    }
    #[test]
    fn tag_and_folder_pages_reject_missing_cursors_duplicates_and_unsafe_ids() {
        let tag = json!({"id":1,"name":"标签","color":null,"file_count":0});
        assert!(
            parse_tags(json!({"tags":[tag.clone()],"previous_cursor":null,"next_cursor":null}))
                .is_ok()
        );
        assert!(parse_tags(json!({"tags":[tag.clone()]})).is_err());
        assert!(
            parse_tags(
                json!({"tags":[tag.clone(),tag.clone()],"previous_cursor":null,"next_cursor":null})
            )
            .is_err()
        );
        let mut invalid = tag;
        invalid["id"] = json!(9_007_199_254_740_992u64);
        assert!(
            parse_tags(json!({"tags":[invalid],"previous_cursor":null,"next_cursor":null}))
                .is_err()
        );
        assert!(
            parse_folders(json!({"folders":["a","b"],"previous_cursor":null,"next_cursor":null}))
                .is_ok()
        );
        assert!(
            parse_folders(json!({"folders":["a","a"],"previous_cursor":null,"next_cursor":null}))
                .is_err()
        );
        assert!(
            parse_folders(json!({"folders":["a/b"],"previous_cursor":null,"next_cursor":null}))
                .is_err()
        );
        assert!(parse_listed(json!({"file_id":null,"tags":[],"tags_has_more":false})).is_ok());
        assert!(
            parse_listed(
                json!({"file_id":9_007_199_254_740_992u64,"tags":[],"tags_has_more":false})
            )
            .is_err()
        );
        assert!(validate_mutation("/scan", "POST", json!({"scanned":"1"})).is_err());
        assert!(validate_mutation("/backup", "POST", json!({"filename":""})).is_err());
    }
}
