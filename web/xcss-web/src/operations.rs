use crate::{
    http::{self, Error},
    model::{self, Dialog, File, Model, Rename},
    protocol::{self, Phase, State},
};
use futures::channel::oneshot;
use serde_json::json;
use std::cell::RefCell;
use wasm_bindgen::JsCast;
use xcss_web::i18n::t;
use xcss_web::prelude::*;

thread_local! {static ANSWER:RefCell<Option<oneshot::Sender<(String,String)>>>=const{RefCell::new(None)};}
thread_local! {static DIALOG_QUEUE:std::rc::Rc<futures::lock::Mutex<()>>=std::rc::Rc::new(futures::lock::Mutex::new(()));}
pub async fn ask(model: Model, dialog: Dialog) -> (String, String) {
    let queue = DIALOG_QUEUE.with(std::rc::Rc::clone);
    let _guard = queue.lock().await;
    let (sender, receiver) = oneshot::channel();
    ANSWER.with(|a| {
        if let Some(old) = a.borrow_mut().replace(sender) {
            let _ = old.send(("cancel".into(), String::new()));
        }
    });
    model.dialog.set(Dialog {
        open: true,
        ..dialog
    });
    receiver
        .await
        .unwrap_or_else(|_| ("cancel".into(), String::new()))
}
pub fn answer(model: Model, choice: &str) {
    if let Some(dialog) = http::document()
        .query_selector(".action-dialog")
        .ok()
        .flatten()
        .and_then(|e| e.dyn_into::<web_sys::HtmlDialogElement>().ok())
        && dialog.open()
    {
        dialog.close();
    }
    let value = model.dialog.get_untracked().value;
    let return_focus = model.dialog.get_untracked().return_focus;
    model.dialog.update(|d| d.open = false);
    if choice == "cancel" || !return_focus.is_empty() {
        let action = model.action.get_untracked();
        let selector = if return_focus.is_empty() {
            if action.is_empty() {
                String::new()
            } else {
                format!("[data-file-action={action}]")
            }
        } else {
            return_focus
        };
        if !selector.is_empty() {
            gloo_timers::callback::Timeout::new(0, move || {
                if let Some(button) = http::document()
                    .query_selector(&selector)
                    .ok()
                    .flatten()
                    .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
                {
                    let _ = button.focus();
                }
            })
            .forget();
        }
    }
    ANSWER.with(|a| {
        if let Some(a) = a.borrow_mut().take() {
            let _ = a.send((choice.to_owned(), value));
        }
    });
}
pub async fn message(model: Model, title: String, message: String) {
    message_at(model, title, message, String::new()).await;
}
async fn message_at(model: Model, title: String, message: String, return_focus: String) {
    let _ = ask(
        model,
        Dialog {
            title,
            message,
            confirm: t("关闭", "Close"),
            return_focus,
            ..Dialog::default()
        },
    )
    .await;
}
async fn confirm_at(
    model: Model,
    title: String,
    message: String,
    confirm: String,
    return_focus: String,
) -> bool {
    ask(
        model,
        Dialog {
            title,
            message,
            confirm,
            return_focus,
            danger: true,
            ..Dialog::default()
        },
    )
    .await
    .0 == "confirm"
}
pub async fn start_rename(model: Model, name: String, created: bool) {
    if model
        .rename
        .get_untracked()
        .as_ref()
        .is_some_and(|e| e.name == name)
    {
        return;
    }
    if !settle(model).await {
        return;
    }
    model.rename.set(Some(Rename {
        name: name.clone(),
        draft: name.rsplit('/').next().unwrap_or(&name).into(),
        error: String::new(),
        pending: false,
        created,
    }));
    gloo_timers::future::TimeoutFuture::new(0).await;
    if let Some(input) = http::document()
        .query_selector(".inline-name-input")
        .ok()
        .flatten()
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
    {
        let _ = input.focus();
        let value = input.value();
        let dir = model
            .file(&name)
            .is_some_and(|f| f.path_type.ends_with("Dir"));
        let caret = if dir {
            value.chars().count()
        } else {
            value
                .rfind('.')
                .filter(|n| *n > 0)
                .map(|n| value[..n].encode_utf16().count())
                .unwrap_or_else(|| value.encode_utf16().count())
        };
        let _ = input.set_selection_range(caret as u32, caret as u32);
    }
}
pub async fn cancel_rename(model: Model, restore: bool) {
    let Some(editor) = model.rename.get_untracked() else {
        return;
    };
    if editor.pending {
        return;
    }
    let is_dir = model
        .file(&editor.name)
        .is_some_and(|f| f.path_type.ends_with("Dir"));
    model.rename.set(None);
    if editor.created {
        model::load(model, true).await;
        if model.file(&editor.name).is_none() {
            model.listing.update(|l| {
                l.status = t(
                    "已创建的项目被当前筛选隐藏。",
                    "The created item is hidden by the current filters.",
                )
            });
        }
    }
    if restore {
        let selector = if editor.created {
            if is_dir { ".new-folder" } else { ".new-file" }
        } else {
            "[data-file-action=rename]"
        };
        if let Some(element) = http::document()
            .query_selector(selector)
            .ok()
            .flatten()
            .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let _ = element.focus();
        }
    }
}
pub async fn settle(model: Model) -> bool {
    settle_impl(model, false).await
}
pub async fn submit_rename(model: Model) -> bool {
    settle_impl(model, true).await
}
async fn settle_impl(model: Model, keep_invalid: bool) -> bool {
    let Some(editor) = model.rename.get_untracked() else {
        return true;
    };
    if editor.pending {
        return false;
    }
    if editor.draft == editor.name.rsplit('/').next().unwrap_or_default() {
        cancel_rename(model, false).await;
        return true;
    }
    if !http::valid_path(&editor.draft) || editor.draft.contains('/') {
        if !keep_invalid {
            cancel_rename(model, false).await;
            return true;
        }
        model.rename.update(|e| {
            if let Some(e) = e {
                e.error = t(
                    "名称不能为空，不能包含斜杠或空字符，最多 255 个 UTF-8 字节。",
                    "Use one non-empty name without '/' or NUL, at most 255 UTF-8 bytes.",
                );
            }
        });
        return false;
    }
    let Some(file) = model.file(&editor.name) else {
        model.rename.set(None);
        return true;
    };
    model.rename.update(|e| {
        if let Some(e) = e {
            e.pending = true;
        }
    });
    let body = json!({"source":model.path(&file.name),"name":editor.draft,"source_revision":file.revision,"overwrite":false});
    let result = relocate(model, "rename", body).await;
    match result {
        Ok(()) => {
            let restore_name = http::document().active_element().is_none_or(|e| {
                e.class_list().contains("inline-name-input") || e.tag_name() == "BODY"
            });
            model.rename.set(None);
            model.invalidate(false);
            model::load(model, true).await;
            if restore_name {
                let name = editor.draft.clone();
                gloo_timers::callback::Timeout::new(0, move || {
                    if let Ok(links) =
                        http::document().query_selector_all(".paths-table .cell-name a")
                    {
                        for index in 0..links.length() {
                            if let Some(link) = links
                                .item(index)
                                .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
                                && link.text_content().as_deref() == Some(&name)
                            {
                                let _ = link.focus();
                                break;
                            }
                        }
                    }
                })
                .forget();
            }
            true
        }
        Err(e) => {
            if !e.auth() {
                message_at(
                    model,
                    t("重命名失败", "Rename failed"),
                    format!(
                        "{} \"{}\" {} \"{}\": {}",
                        t("无法重命名", "Unable to rename"),
                        model.path(&editor.name),
                        t("到", "to"),
                        model.path(&editor.draft),
                        e.message
                    ),
                    "[data-file-action=rename]".into(),
                )
                .await;
            }
            if e.status == 412 || e.unknown || e.auth() {
                model.rename.set(None);
                model.invalidate(e.unknown);
                if e.status == 412 {
                    model::load(model, true).await;
                }
            } else {
                let text = e.message.clone();
                model.rename.update(|e| {
                    if let Some(e) = e {
                        e.pending = false;
                        e.error = result_message(&text);
                    }
                });
            }
            false
        }
    }
}
fn result_message(message: &str) -> String {
    if message.is_empty() {
        t("重命名失败，请重试。", "Rename failed. Try again.")
    } else {
        message.to_owned()
    }
}
async fn relocate(model: Model, endpoint: &str, body: serde_json::Value) -> Result<(), Error> {
    let key = format!("path:{}", body["source"].as_str().unwrap_or_default());
    if !model.begin(&key, t("操作进行中…", "Operation in progress…")) {
        return Err(Error::message(t("此项目正在操作。", "This item is busy.")));
    }
    model.control_begin(endpoint);
    let mut result = http::mutation(
        &format!("/__xczs__/api/{endpoint}"),
        "POST",
        Some(body.clone()),
        vec![],
    )
    .await;
    if let Err(e) = &result
        && e.trusted(409, "destination_exists")
        && let Some(revision) = &e.target_revision
        && confirm_at(
            model,
            t("覆盖目标？", "Overwrite destination?"),
            {
                let source = body["source"].as_str().unwrap_or_default();
                let directory = if endpoint == "rename" {
                    source.rsplit_once('/').map(|(d, _)| d).unwrap_or_default()
                } else {
                    body["directory"]
                        .as_str()
                        .unwrap_or_default()
                        .trim_end_matches('/')
                };
                let name = if endpoint == "rename" {
                    body["name"].as_str().unwrap_or_default()
                } else {
                    source.rsplit('/').next().unwrap_or_default()
                };
                let destination = format!("{directory}/{name}");
                if xcss_web::i18n::locale() == "en" {
                    format!("Replace \"{destination}\" with \"{source}\"?")
                } else {
                    format!("将“{destination}”替换为“{source}”？")
                }
            },
            t("覆盖", "Overwrite"),
            format!("[data-file-action={endpoint}]"),
        )
        .await
    {
        let mut body = body;
        body["overwrite"] = json!(true);
        body["destination_revision"] = json!(revision);
        result = http::mutation(
            &format!("/__xczs__/api/{endpoint}"),
            "POST",
            Some(body),
            vec![],
        )
        .await;
    }
    model.control_end(endpoint);
    model.end(&key);
    result.map(|_| ())
}
pub async fn delete(model: Model, file: File) {
    if !confirm_at(
        model,
        t("删除项目", "Delete item"),
        format!(
            "{} \"{}\"? {}",
            t("删除", "Delete"),
            file.name,
            t("此操作无法撤销。", "This action cannot be undone.")
        ),
        t("删除", "Delete"),
        "[data-file-action=delete]".into(),
    )
    .await
    {
        return;
    }
    let key = format!("path:{}", model.path(&file.name));
    if !model.begin(
        &key,
        format!("{} {}…", t("正在删除", "Deleting"), file.name),
    ) {
        return;
    }
    model.control_begin("delete");
    match http::mutation(
        &http::file_url(&model.path(&file.name)),
        "DELETE",
        None,
        vec![("If-Match".into(), format!("\"{}\"", file.revision))],
    )
    .await
    {
        Ok(_) => {
            model
                .listing
                .update(|l| l.files.retain(|f| f.name != file.name));
            model.invalidate(false);
            model::load(model, true).await;
            gloo_timers::callback::Timeout::new(0, move || {
                if model.rename.get_untracked().is_some() {
                    return;
                }
                if let Some(button) = http::document()
                    .query_selector("[data-file-action=delete]")
                    .ok()
                    .flatten()
                    .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
                {
                    let _ = button.focus();
                }
            })
            .forget();
        }
        Err(e) => {
            if e.unknown || e.status == 412 {
                model.invalidate(e.unknown);
            }
            if !e.auth() {
                message_at(
                    model,
                    t("删除失败", "Delete failed"),
                    format!(
                        "{} \"{}\": {}",
                        t("无法删除", "Unable to delete"),
                        file.name,
                        e.message
                    ),
                    "[data-file-action=delete]".into(),
                )
                .await;
            }
            if e.status == 412 {
                model::load(model, true).await;
            }
        }
    }
    model.control_end("delete");
    model.end(&key);
}
pub async fn move_file(model: Model, file: File) {
    if !settle(model).await {
        return;
    }
    let (choice,directory)=ask(model,Dialog{title:t("移动项目","Move item"),message:t("请输入共享根目录下已有的目标文件夹，项目名称将保持不变。","Enter an existing destination folder under the shared root. The item name will not change."),label:t("目标文件夹","Destination folder"),value:model.directory.get_untracked(),confirm:t("移动","Move"),return_focus:"[data-file-action=move]".into(),..Dialog::default()}).await;
    if choice != "confirm" || directory.is_empty() {
        return;
    }
    let directory = format!("/{}", directory.trim_matches('/'));
    if directory != "/" && !http::valid_path(&directory[1..]) {
        message(
            model,
            t("移动失败", "Move failed"),
            t(
                "目标文件夹路径无效。",
                "The destination folder path is invalid.",
            ),
        )
        .await;
        return;
    }
    let body = json!({"source":model.path(&file.name),"directory":directory,"source_revision":file.revision,"overwrite":false});
    match relocate(model, "move", body).await {
        Ok(()) => {
            let _ = http::window()
                .location()
                .set_href(&http::file_url(&directory));
        }
        Err(e) => {
            if e.unknown || e.status == 412 {
                model.invalidate(e.unknown);
            }
            if !e.auth() {
                let destination = format!("{}/{}", directory.trim_end_matches('/'), file.name);
                message_at(
                    model,
                    t("移动失败", "Move failed"),
                    format!(
                        "{} \"{}\" {} \"{}\": {}",
                        t("无法移动", "Unable to move"),
                        model.path(&file.name),
                        t("到", "to"),
                        destination,
                        e.message
                    ),
                    "[data-file-action=move]".into(),
                )
                .await;
            }
            if e.status == 412 {
                model::load(model, true).await;
            }
        }
    }
}
pub async fn create(model: Model, is_dir: bool) {
    if !settle(model).await {
        return;
    }
    if !model.begin(
        "create-item",
        if is_dir {
            t("正在创建新文件夹…", "Creating a new folder…")
        } else {
            t("正在创建新空文件…", "Creating a new empty file…")
        },
    ) {
        return;
    }
    let control = if is_dir {
        "create-folder"
    } else {
        "create-file"
    };
    model.control_begin(control);
    model.action.set(String::new());
    model.tag_target.set(None);
    let base = if is_dir { "newfolder" } else { "newfile" };
    let mut exhausted = true;
    for attempt in 0..1000 {
        let name = if attempt == 0 {
            base.to_owned()
        } else {
            format!("{base} ({})", attempt + 1)
        };
        let path = model.path(&name);
        let result = if is_dir {
            http::mutation(
                "/__xczs__/api/mkdir",
                "POST",
                Some(json!({"path":path})),
                vec![],
            )
            .await
            .map(|_| ())
        } else {
            create_empty(&path).await
        };
        match result {
            Ok(()) => {
                exhausted = false;
                model.rename.set(None);
                model.invalidate(false);
                model.dir_exists.set(true);
                match http::preflight(&[path]).await {
                    Ok(targets) if targets[0].exists => {
                        let revision = targets[0].revision.clone().unwrap();
                        model.listing.update(|l| {
                            l.files.retain(|f| f.name != name);
                            l.files.insert(
                                0,
                                File {
                                    path_type: if is_dir { "Dir" } else { "File" }.into(),
                                    name: name.clone(),
                                    mtime: js_sys::Date::now() as u64,
                                    size: 0,
                                    revision,
                                    tags: vec![],
                                    file_id: None,
                                    tags_has_more: false,
                                    tags_available: true,
                                },
                            );
                            l.window_start = 0;
                        });
                        model.control_end(control);
                        model.end("create-item");
                        start_rename(model, name, true).await;
                        return;
                    }
                    _ => model::load(model, true).await,
                }
                break;
            }
            Err(e) if is_dir && e.trusted(409, "path_exists") => continue,
            Err(e)
                if !is_dir
                    && e.trusted(409, "destination_exists")
                    && e.operation_state != "unknown" =>
            {
                continue;
            }
            Err(e) => {
                exhausted = false;
                if e.unknown {
                    model.invalidate(true);
                }
                if !e.auth() {
                    message(
                        model,
                        if is_dir {
                            t("创建文件夹失败", "Create folder failed")
                        } else {
                            t("创建文件失败", "Create file failed")
                        },
                        e.message,
                    )
                    .await;
                }
                break;
            }
        }
    }
    if exhausted {
        message(
            model,
            if is_dir {
                t("创建文件夹失败", "Create folder failed")
            } else {
                t("创建文件失败", "Create file failed")
            },
            if xcss_web::i18n::locale() == "en" {
                format!(
                    "No available default {} name was found after 1000 attempts.",
                    if is_dir { "folder" } else { "file" }
                )
            } else {
                format!(
                    "尝试 1000 次后仍未找到可用的默认{}名称。",
                    if is_dir { "文件夹" } else { "文件" }
                )
            },
        )
        .await;
    }
    model.control_end(control);
    model.end("create-item");
}
async fn create_empty(path: &str) -> Result<(), Error> {
    let id = http::uuid();
    let response = http::put_empty(path, &id).await;
    match response {
        Ok(r) => {
            let b = protocol::classify(Phase::Fresh, r.status, |n| r.header(n), &id, 0);
            if b.as_ref().is_some_and(|b| b.state == State::Committed) {
                return Ok(());
            }
            let e = r.error(
                b.is_none()
                    || b.as_ref()
                        .is_some_and(|b| matches!(b.state, State::Unknown | State::Running)),
            );
            if e.auth() {
                return Err(e);
            }
            if b.as_ref()
                .is_some_and(|b| matches!(b.state, State::NotStarted | State::Rejected))
                && e.trusted(409, "destination_exists")
            {
                return Err(e);
            }
            if b.as_ref()
                .is_some_and(|b| b.state == State::AwaitingConfirmation)
                && e.trusted(409, "destination_exists")
            {
                discard(path, &id, 0).await?;
                return Err(e);
            }
            if !e.unknown {
                return Err(e);
            }
            reconcile_empty(path, &id, e).await
        }
        Err(e) if e.unknown => reconcile_empty(path, &id, e).await,
        Err(e) => Err(e),
    }
}
async fn reconcile_empty(path: &str, id: &str, original: Error) -> Result<(), Error> {
    let h = http::raw(
        &http::file_url(path),
        "HEAD",
        vec![(http::UP_ID.into(), id.into())],
        None,
        None,
        30_000,
        false,
    )
    .await?;
    match protocol::classify(Phase::Checkpoint, h.status, |n| h.header(n), id, 0).map(|b| b.state) {
        Some(State::Committed) => Ok(()),
        Some(State::AwaitingConfirmation) => {
            discard(path, id, 0).await?;
            Err(Error {
                status: 409,
                problem_status: 409,
                code: "destination_exists".into(),
                operation_state: "rejected".into(),
                message: t("路径已存在", "Path already exists"),
                ..Error::default()
            })
        }
        _ => Err(original),
    }
}

pub async fn discard(path: &str, id: &str, length: u64) -> Result<(), Error> {
    let r = http::json_request(
        "/__xczs__/api/upload/discard",
        "POST",
        Some(json!({"path":path,"upload_id":id})),
        vec![],
        true,
    )
    .await?;
    if !r.ok() {
        let error = r.error(true);
        if error.auth() {
            return Err(error);
        }
    }
    if protocol::classify(Phase::Discard, r.status, |n| r.header(n), id, length).is_some() {
        Ok(())
    } else {
        Err(Error {
            message: t(
                "无法确认暂存上传的清理结果。",
                "Could not confirm cleanup of the staged upload.",
            ),
            unknown: true,
            ..Error::default()
        })
    }
}

#[component]
pub fn ActionDialog(model: Model) -> impl IntoView {
    let node = NodeRef::<xcss_web::html::Dialog>::new();
    Effect::new(move |_| {
        let state = model.dialog.get();
        if let Some(dialog) = node.get() {
            if state.open && !dialog.open() {
                let _ = dialog.show_modal();
                let selector = if state.label.is_empty() {
                    ".action-dialog-confirm"
                } else {
                    "#action-dialog-input"
                };
                if let Some(input) = dialog
                    .query_selector(selector)
                    .ok()
                    .flatten()
                    .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
                {
                    let _ = input.focus();
                }
            } else if !state.open && dialog.open() {
                dialog.close();
            }
        }
    });
    View::new(
    xcss_web::html::dialog()
        .child(
            #[allow(unused_braces)]
            {
                xcss_web::html::form()
                    .child((
                        xcss_web::html::h2()
                            .child(
                                #[allow(unused_braces)]
                                {
                                    IntoRender::into_render({
                                        move || model.dialog.get().title
                                    })
                                },
                            )
                            .id("action-dialog-title"),
                        xcss_web::html::p()
                            .child(
                                #[allow(unused_braces)]
                                {
                                    IntoRender::into_render({
                                        move || model.dialog.get().message
                                    })
                                },
                            )
                            .id("action-dialog-message"),
                        xcss_web::html::div()
                            .child((
                                xcss_web::html::label()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({
                                                move || model.dialog.get().label
                                            })
                                        },
                                    )
                                    .r#for("action-dialog-input"),
                                xcss_web::html::input()
                                    .id("action-dialog-input")
                                    .r#type("text")
                                    .autocomplete("off")
                                    .prop("value", move || model.dialog.get().value)
                                    .on(
                                        xcss_web::ev::input,
                                        move |e| {
                                            model.dialog.update(|d| d.value = event_target_value(&e))
                                        },
                                    ),
                            ))
                            .class("action-dialog-input-group")
                            .hidden(
                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                model.dialog.get().label.is_empty()),
                            ),
                        xcss_web::html::label()
                            .child((
                                IntoRender::into_render({
                                    t("颜色", "Color")
                                }),
                                xcss_web::html::input()
                                    .r#type("color")
                                    .prop("value", move || model.dialog.get().color)
                                    .on(
                                        xcss_web::ev::input,
                                        move |e| {
                                            model.dialog.update(|d| d.color = event_target_value(&e))
                                        },
                                    ),
                            ))
                            .hidden(
                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                model.dialog.get().color.is_empty()),
                            ),
                        xcss_web::html::div()
                            .child((
                                xcss_web::html::button()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({
                                                move || {
                                                    let label = model.dialog.get().cancel;
                                                    if label.is_empty() { t("取消", "Cancel") } else { label }
                                                }
                                            })
                                        },
                                    )
                                    .class("action-dialog-cancel xcss-button")
                                    .r#type("button")
                                    .hidden(
                                        ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                        model.dialog.get().confirm == t("关闭", "Close")),
                                    )
                                    .on(
                                        xcss_web::ev::click,
                                        move |_| answer(model, "cancel"),
                                    ),
                                xcss_web::html::button()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({
                                                move || model.dialog.get().alternate
                                            })
                                        },
                                    )
                                    .class("action-dialog-alternate xcss-button")
                                    .r#type("button")
                                    .hidden(
                                        ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                        model.dialog.get().alternate.is_empty()),
                                    )
                                    .on(
                                        xcss_web::ev::click,
                                        move |_| answer(model, "alternate"),
                                    ),
                                xcss_web::html::button()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({
                                                move || model.dialog.get().confirm
                                            })
                                        },
                                    )
                                    .class("action-dialog-confirm xcss-button")
                                    .r#type("submit")
                                    .class(("xcss-danger", move || model.dialog.get().danger)),
                            ))
                            .class("action-dialog-actions"),
                    ))
                    .class("action-dialog-form")
                    .method("dialog")
                    .on(
                        xcss_web::ev::submit,
                        move |e| {
                            e.prevent_default();
                            answer(model, "confirm");
                        },
                    )
            },
        )
        .class("action-dialog xcss-dialog")
        .node_ref(node)
        .aria_labelledby("action-dialog-title")
        .aria_describedby("action-dialog-message")
        .on(
            xcss_web::ev::Custom::new("cancel"),
            move |e: web_sys::Event| {
                e.prevent_default();
                answer(model, "cancel");
            },
        ),
)
}
