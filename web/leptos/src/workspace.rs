use crate::{
    http,
    model::{self, File, Model},
    operations, tags, upload,
};
use leptos::prelude::*;
use wasm_bindgen::{JsCast, closure::Closure};
use xcss_web::i18n::t;

pub fn apply_filters(model: Model) {
    leptos::task::spawn_local(async move {
        if !operations::settle(model).await {
            return;
        }
        let query = model.filters.get_untracked().query();
        let path = http::window()
            .location()
            .pathname()
            .unwrap_or_else(|_| "/".into());
        let _ = http::window().history().and_then(|h| {
            h.push_state_with_url(
                &wasm_bindgen::JsValue::NULL,
                "",
                Some(&format!("{path}?{query}#files")),
            )
        });
        model.view.set("files".into());
        model::load(model, true).await;
    });
}
fn activate(model: Model, file: File) {
    let mode = model.action.get_untracked();
    if matches!(mode.as_str(), "rename" | "delete" | "move") {
        model.action.set(String::new());
    }
    match mode.as_str() {
        "rename" => leptos::task::spawn_local(async move {
            operations::start_rename(model, file.name, false).await;
        }),
        "delete" => leptos::task::spawn_local(async move {
            operations::delete(model, file).await;
        }),
        "move" => leptos::task::spawn_local(async move {
            operations::move_file(model, file).await;
        }),
        "download" => {
            if file.path_type.ends_with("Dir") {
                model.notice.set(t(
                    "文件夹不能直接下载，请选择文件。",
                    "Folders cannot be downloaded. Choose a file.",
                ));
                return;
            }
            let a = http::document().create_element("a").unwrap();
            let _ = a.set_attribute("href", &http::file_url(&model.path(&file.name)));
            let _ = a.set_attribute("download", &file.name);
            if let Some(a) = a.dyn_ref::<web_sys::HtmlElement>() {
                a.click();
            }
            model.action.set(String::new());
        }
        "tags" => {
            if file.path_type.ends_with("Dir") {
                model.notice.set(t(
                    "文件夹不支持标签，请选择文件。",
                    "Folders do not support tags. Choose a file.",
                ));
                return;
            }
            model.tag_target.set(Some(file));
        }
        _ => {}
    }
}
#[component]
pub fn Workspace(model: Model) -> impl IntoView {
    let folder_input = NodeRef::<leptos::html::Input>::new();
    Effect::new(move |_| {
        if let Some(input) = folder_input.get() {
            let _ = input.set_attribute("webkitdirectory", "");
        }
    });
    let history = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        let previous = model.view.get_untracked();
        model.view.set(model::view());
        if previous != model.view.get_untracked() {
            model.action.set(String::new());
            model.tag_target.set(None);
        }
        let filters = model::Filters::location();
        let changed = filters != model.filters.get_untracked();
        if changed {
            model.filters.set(filters);
            leptos::task::spawn_local(async move {
                model::load(model, true).await;
            });
        } else if model.view.get_untracked() == "files"
            && !model.listing.with_untracked(|l| l.loaded)
        {
            leptos::task::spawn_local(async move {
                model::load(model, false).await;
            });
        }
    });
    let _ = http::window()
        .add_event_listener_with_callback("hashchange", history.as_ref().unchecked_ref());
    let _ = http::window()
        .add_event_listener_with_callback("popstate", history.as_ref().unchecked_ref());
    history.forget();
    let auth = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        if xcss_web::api::snapshot().account_updated {
            let _ = http::window().location().replace("/__xczs__/login");
        }
    });
    let _ = http::window().add_event_listener_with_callback(
        xcss_web::api::AUTH_CHANGE_EVENT,
        auth.as_ref().unchecked_ref(),
    );
    auth.forget();
    let keys =
        Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |e: web_sys::KeyboardEvent| {
            if e.key() == "Escape"
                && !e.is_composing()
                && model.rename.get_untracked().is_none()
                && !model.dialog.get_untracked().open
            {
                model.action.set(String::new());
                model.tag_target.set(None);
            }
        });
    let _ =
        http::document().add_event_listener_with_callback("keydown", keys.as_ref().unchecked_ref());
    keys.forget();
    for name in ["dragover", "drop"] {
        let drop = Closure::<dyn FnMut(web_sys::DragEvent)>::new(|e: web_sys::DragEvent| {
            if let Some(data) = e.data_transfer()
                && data
                    .types()
                    .iter()
                    .any(|v| v.as_string().as_deref() == Some("Files"))
            {
                e.prevent_default();
                data.set_drop_effect("none");
            }
        });
        let _ =
            http::document().add_event_listener_with_callback(name, drop.as_ref().unchecked_ref());
        drop.forget();
    }
    Effect::new(move |_| {
        let view = model.view.get();
        let title = if view == "tags" || view == "status" {
            t("xczs 标签库", "xczs Tag Library")
        } else {
            format!(
                "{} - {}",
                model.directory.get(),
                t("xczs 文件管理", "xczs File Manager")
            )
        };
        http::document().set_title(&title);
    });
    let tag_version = StoredValue::new(model.tags_version.get_untracked());
    Effect::new(move |_| {
        let next = model.tags_version.get();
        if next != tag_version.get_value() {
            tag_version.set_value(next);
            if model.listing.with_untracked(|l| l.loaded) {
                leptos::task::spawn_local(async move {
                    model::load(model, true).await;
                });
            }
        }
    });
    if model.view.get_untracked() == "files" {
        leptos::task::spawn_local(async move {
            model::load(model, false).await;
        });
    }
    view! {<a class="xcss-skip-link" href="#file-content" on:click=move|e|{e.prevent_default();if let Some(el)=http::document().get_element_by_id("file-content").and_then(|e|e.dyn_into::<web_sys::HtmlElement>().ok()){let _=el.focus();}}>{t("跳转到文件内容","Skip to files")}</a>
    <header class="xcss-page-header"><div class="xcss-header-navigation-slot"><div class="xcss-header-brand-navigation"><strong class="xcss-product-identity">"xczs"</strong><nav class="xcss-header-navigation" aria-label=t("主导航","Main navigation")>
    {[("files",t("文件","Files")),("tags",t("标签管理","Manage tags")),("status",t("服务状态","Service status"))].into_iter().map(move|(id,label)|view!{<a href=format!("#{id}") aria-current=move||(model.view.get()==id).then_some("page")>{label}</a>}).collect_view()}</nav></div></div>
    <div class="toolbox-right xcss-header-actions" role="group" aria-label=t("全局操作","Global actions")><button type="button" class="xcss-button xcss-icon-button" aria-label=t("重新载入页面","Reload page") on:click=move|_|{let _=http::window().location().reload();}><Icon name="refresh"/></button><xcss_web::components::LanguageToggle/><xcss_web::components::ThemeToggle/><button class="logout-btn xcss-button xcss-icon-button" aria-label=t("退出","Sign out") on:click=move|_|{leptos::task::spawn_local(async move{match xcss_web::api::logout().await{Ok(_)=>{let _=http::window().location().set_href("/__xczs__/login");},Err(e)=>operations::message(model,t("退出失败","Sign out failed"),e.to_string()).await}});} ><Icon name="logout"/><span class="user-name" hidden>{xcss_web::api::current_session().map(|s|s.username).unwrap_or_default()}</span></button><button type="button" class="xcss-button xcss-icon-button xcss-account-entry" data-xcss-server-corner="none" style:border-radius="50%" style:aspect-ratio="1" on:click=move|_|{let _=http::window().location().set_hash("account");} aria-label=t("账号设置","Account settings")><Icon name="account"/></button></div></header>
    <main class="main xcss-shell-main" id="file-content" tabindex="-1" data-workspace-view=move||model.view.get()>
    <Show when=move||model.view.get()=="unknown"><p class="workspace-navigation-error" role="alert">{t("页面不存在，请选择上方菜单。","Page not found. Choose a menu above.")}</p></Show>
    <div class="xczs-file-actions xcss-secondary-navigation" role="group" aria-label=t("文件操作","File controls")><nav class="xczs-root-navigation" aria-label=t("根目录导航","Root directory navigation")><a href="/" aria-label=t("根目录","Root") title=t("根目录","Root")><Icon name="home"/></a></nav>
    <button type="button" class="control upload-file xcss-button xcss-icon-button" aria-label=t("上传文件","Upload files") title=t("上传文件","Upload files") on:click=move|_|click_input("file")><Icon name="upload"/></button><input class="visually-hidden" type="file" id="file" name="file" aria-label=t("选择要上传的文件","Choose files to upload") tabindex="-1" multiple on:change=move|e|{let input=event_target::<web_sys::HtmlInputElement>(&e);upload::select(model,input.files());input.set_value("");}/>
    <button type="button" class="control upload-folder xcss-button xcss-icon-button" aria-label=t("上传文件夹","Upload folder") title=t("上传文件夹","Upload folder") on:click=move|_|click_input("folder")><Icon name="folder"/></button><input class="visually-hidden" type="file" id="folder" name="folder" aria-label=t("选择要上传的文件夹","Choose a folder to upload") tabindex="-1" node_ref=folder_input multiple=true on:change=move|e|{let input=event_target::<web_sys::HtmlInputElement>(&e);upload::select(model,input.files());input.set_value("");}/>
    <button type="button" class="control new-folder xcss-button xcss-icon-button" aria-label=t("新建文件夹","New folder") title=t("新建文件夹","New folder") aria-busy=move||model.busy_actions.with(|m|m.get("create-folder").copied().unwrap_or(0)>0).to_string() aria-disabled=move||model.busy_actions.with(|m|m.get("create-folder").copied().unwrap_or(0)>0).to_string() on:click=move|_|leptos::task::spawn_local(async move{operations::create(model,true).await;})><Icon name="create"/></button>
    <button type="button" class="control new-file xcss-button xcss-icon-button" aria-label=t("新建空文件","New empty file") title=t("新建空文件","New empty file") aria-busy=move||model.busy_actions.with(|m|m.get("create-file").copied().unwrap_or(0)>0).to_string() aria-disabled=move||model.busy_actions.with(|m|m.get("create-file").copied().unwrap_or(0)>0).to_string() on:click=move|_|leptos::task::spawn_local(async move{operations::create(model,false).await;})><Icon name="file"/></button>
    {[("move",t("移动","Move")),("download",t("下载","Download")),("delete",t("删除","Delete")),("rename",t("重命名","Rename")),("tags",t("标签","Tags"))].into_iter().map(move|(mode,label)|view!{<button type="button" class="control file-action-mode xcss-button xcss-icon-button" data-file-action=mode aria-label=label.clone() title=label aria-busy=move||model.busy_actions.with(|m|m.get(mode).copied().unwrap_or(0)>0).then_some("true") aria-disabled=move||model.busy_actions.with(|m|m.get(mode).copied().unwrap_or(0)>0).then_some("true") aria-pressed=move||(model.action.get()==mode).to_string() on:click=move|_|{if model.busy_actions.with_untracked(|m|m.get(mode).copied().unwrap_or(0)>0){return;}model.notice.set(String::new());model.action.update(|a|*a=if a==mode{String::new()}else{mode.into()});if mode!="tags"{model.tag_target.set(None);}}><Icon name=mode/></button>}).collect_view()}
    <form class="searchbar" aria-label=t("搜索文件和标签","Search files and tags") on:submit=move|e|{e.prevent_default();let form=event_target::<web_sys::HtmlFormElement>(&e);if let Ok(data)=web_sys::FormData::new_with_form(&form){model.filters.update(|f|{f.q=data.get("q").as_string().unwrap_or_default();f.all=data.get_all("all").iter().filter_map(|v|v.as_string()).collect::<Vec<_>>().join(",");f.any=data.get_all("any").iter().filter_map(|v|v.as_string()).collect::<Vec<_>>().join(",");f.exclude=data.get_all("exclude").iter().filter_map(|v|v.as_string()).collect::<Vec<_>>().join(",");});apply_filters(model);}}><div class="search-input"><label class="visually-hidden" for="search">{t("搜索文件或文件夹","Search files or folders")}</label><input id="search" name="q" type="text" maxlength="128" autocomplete="off" aria-label=t("搜索文件或文件夹","Search files or folders") placeholder=t("搜索文件或文件夹","Search files or folders") prop:value=move||model.filters.get().q/></div><tags::TagFilters model/><button class="search-submit xcss-button" type="submit" aria-label=t("搜索","Search")>"↵"</button></form></div>
    <div class="file-action-hint" hidden=move||model.action.get().is_empty()><span id="file-action-hint" role="status">{move||{let notice=model.notice.get();if !notice.is_empty(){return notice;}match model.action.get().as_str(){"move"=>t("请选择要移动的文件或文件夹。","Choose a file or folder to move."),"download"=>t("请选择要下载的文件。","Choose a file to download."),"delete"=>t("请选择要删除的文件或文件夹。","Choose a file or folder to delete."),"rename"=>t("请选择要重命名的文件或文件夹。","Choose a file or folder to rename."),"tags"=>t("请选择文件，然后点击下方标签添加或移除。","Choose a file, then click a tag below to add or remove it."),_=>String::new()}}}</span><button class="xcss-button" on:click=move|_|{model.action.set(String::new());model.tag_target.set(None);} >{t("取消","Cancel")}</button></div>
    <tags::FileTagToolbar model/><section class="xcss-content-panel file-toolbar" aria-label=t("文件操作","File controls")><Breadcrumb model/></section>
    <div class="index-page xcss-content-panel"><div class="operation-status" role="status" aria-live="polite" hidden=move||model.operation_status.get().is_empty()>{move||model.operation_status.get()}</div><upload::Queue model/>
    <div class="empty-folder" role="status" hidden=move||model.listing.with(|l|!l.loaded||!l.files.is_empty())>{move||if model.filters.with(|f|!f.q.is_empty()||!f.all.is_empty()||!f.any.is_empty()||!f.exclude.is_empty()){t("没有搜索结果","No search results")}else if model.dir_exists.get(){t("文件夹为空","Folder is empty")}else{t("上传文件后将自动创建此文件夹","Uploading files will create this folder automatically")}}</div>
    <table class="paths-table xcss-table" data-action-mode=move||model.action.get() aria-label=t("文件列表","File list") hidden=move||model.listing.with(|l|l.files.is_empty()) class:has-inline-editor=move||model.rename.get().is_some()><colgroup><col class="file-icon"/><col/><col class="file-tags"/><col class="file-modified"/><col class="file-size"/></colgroup><thead><tr>
    {[ ("name",t("名称","Name")),("mtime",t("修改时间","Last modified")),("size",t("大小","Size"))].into_iter().map(move|(key,label)|{let tags=(key=="name").then(||view!{<th class="cell-tags" scope="col">{t("标签","Tags")}</th>});view!{<th class=format!("cell-{key}") colspan=if key=="name"{2}else{1} scope="col" aria-sort=move||{let f=model.filters.get();if f.sort==key{Some(if f.order=="asc"{"ascending"}else{"descending"})}else{None}}><a href=move||{let mut f=model.filters.get();f.order=if f.sort==key&&f.order=="asc"{"desc"}else{"asc"}.into();f.sort=key.into();format!("?{}",f.query())} on:click=move|e|{e.prevent_default();model.filters.update(|f|{f.order=if f.sort==key&&f.order=="asc"{"desc"}else{"asc"}.into();f.sort=key.into();});apply_filters(model);}>{label.clone()}<span aria-hidden="true">{move||{let f=model.filters.get();if f.sort==key{if f.order=="asc"{" ↑"}else{" ↓"}}else{""}}}</span></a></th>{tags}}}).collect_view()}
    </tr></thead><tbody><For each=move||model.listing.with(|l|l.files.iter().skip(l.window_start).take(200).cloned().collect::<Vec<_>>()) key=|f|(f.name.clone(),f.revision.clone(),serde_json::to_string(&f.tags).unwrap_or_default(),f.tags_available,f.tags_has_more) children=move|file|view!{<FileRow model file/>}/></tbody></table>
    <div class="list-pagination"><div class="list-status" role="status" aria-live="polite" tabindex="-1">{move||model::listing_status(&model.listing.get())}</div><button class="load-more xcss-button" hidden=move||model.listing.with(|l|l.window_start==0) on:click=move|_|window_move(model,false)>{t("显示前面的项目","Show previous items")}</button><button class="load-more xcss-button" hidden=move||model.listing.with(|l|l.window_start+200>=l.files.len()) on:click=move|_|window_move(model,true)>{t("显示后面的项目","Show next items")}</button><button class="load-more xcss-button" hidden=move||model.listing.with(|l|(l.next.is_none()||l.window_start+200<l.files.len())&&!l.invalidated&&!l.retry) disabled=move||model.listing.get().loading on:click=move|_|leptos::task::spawn_local(async move{model::load(model,false).await;})>{move||if model.listing.get().retry{t("重试","Retry")}else if model.listing.get().invalidated{t("刷新","Refresh")}else{t("加载更多","Load more")}}</button></div></div>
    <tags::Library model/><Show when=move||model.view.get()=="account"><xcss_web::components::AccountPage/></Show></main><operations::ActionDialog model/>}
}
fn click_input(id: &str) {
    if let Some(input) = http::document()
        .get_element_by_id(id)
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
    {
        input.click();
    }
}
#[component]
fn Breadcrumb(model: Model) -> impl IntoView {
    let dir = model.directory.get_untracked();
    let parts = dir
        .trim_start_matches('/')
        .split('/')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let len = parts.len();
    let mut path = String::new();
    view! {<nav class="breadcrumb" aria-label=t("当前文件夹","Current folder") title=dir><a href="/">"/"</a>{parts.into_iter().enumerate().map(|(i,name)|{path.push('/');path.push_str(&http::encode(&name));let href=path.clone();view!{<span>{if i+1==len{view!{<b aria-current="page">{name}</b>}.into_any()}else{view!{<a href=href>{name}</a><span class="separator" aria-hidden="true">"/"</span>}.into_any()}}</span>}}).collect_view()}</nav>}
}
#[component]
fn FileRow(model: Model, file: File) -> impl IntoView {
    let name = StoredValue::new(file.name.clone());
    let f = StoredValue::new(file.clone());
    let mut href = http::file_url(&model.path(&file.name));
    if file.path_type.ends_with("Dir") {
        href.push('/');
    }
    let icon = if file.path_type.ends_with("Dir") {
        "folder"
    } else {
        "file"
    };
    let label = t("重命名", "Rename");
    view! {<tr id=move||format!("addPath{}",model.listing.with(|l|l.files.iter().position(|f|f.name==name.get_value()).unwrap_or(0))) data-index=move||model.listing.with(|l|l.files.iter().position(|f|f.name==name.get_value()).unwrap_or(0)) class:is-tag-selected=move||model.tag_target.get().is_some_and(|f|f.name==name.get_value()) class:is-renaming=move||model.rename.with(|e|e.as_ref().is_some_and(|e|e.name==name.get_value())) on:click=move|e|{let inside_control=e.target().and_then(|e|e.dyn_into::<web_sys::Element>().ok()).is_some_and(|e|e.closest("a,input,button,select,textarea").ok().flatten().is_some());if !inside_control{activate(model,f.get_value());}}><td class="path cell-icon"><Icon name=icon/></td><td class="path cell-name">
        <Show when=move||model.rename.with(|e|e.as_ref().is_some_and(|e|e.name==name.get_value())) fallback=move||{let href=href.clone();view!{<a href=href aria-describedby=move||(!model.action.get().is_empty()).then_some("file-action-hint") download=(!f.get_value().path_type.ends_with("Dir")).then(||name.get_value()) on:keydown=move|e|{if !e.is_composing()&&!model.action.get_untracked().is_empty()&&matches!(e.key().as_str(),"Enter"|" "){e.prevent_default();activate(model,f.get_value());}} on:click=move|e|{if !model.action.get_untracked().is_empty(){e.prevent_default();activate(model,f.get_value());}}>{name.get_value()}</a>}}>
        <input class="inline-name-input" type="text" autocomplete="off" spellcheck="false" aria-label=format!("{label} {}",file.name) aria-describedby=move||format!("inline-error-{}",http::encode(&name.get_value())) prop:value=move||model.rename.get().map(|e|e.draft).unwrap_or_default() disabled=move||model.rename.get().is_some_and(|e|e.pending) aria-busy=move||model.rename.get().is_some_and(|e|e.pending).then_some("true") aria-invalid=move||model.rename.get().is_some_and(|e|!e.error.is_empty()).to_string() on:input=move|e|model.rename.update(|r|if let Some(r)=r{r.draft=event_target_value(&e);r.error.clear();}) on:keydown=move|e|{if e.is_composing(){return;}
    if e.key()=="Escape"{e.prevent_default();leptos::task::spawn_local(async move{operations::cancel_rename(model,true).await;});}else if e.key()=="Enter"{e.prevent_default();leptos::task::spawn_local(async move{operations::submit_rename(model).await;});}} on:blur=move|_|{if model.rename.with_untracked(|r|r.as_ref().is_some_and(|r|r.name==name.get_value())){leptos::task::spawn_local(async move{operations::settle(model).await;});}}/>
        <span class="inline-name-error" id=move||format!("inline-error-{}",http::encode(&name.get_value())) role="alert" hidden=move||model.rename.get().is_none_or(|e|e.error.is_empty())>{move||model.rename.get().map(|e|e.error).unwrap_or_default()}</span></Show></td>
        <td class="cell-tags" data-label=t("标签","Tags")>{file.tags.iter().map(|tag|view!{<span class="file-tag-label" style:border-inline-start-color=tag.color.clone().unwrap_or_default()>{tag.name.clone()}</span>}).collect_view()}{(file.tags.is_empty()&&file.tags_available).then_some("—")}{file.tags_has_more.then(||view!{<span>"…"</span>})}{(!file.tags_available).then(||view!{<span>{t("标签不可用","Tags unavailable")}</span>})}</td><td class="cell-mtime">{model::mtime(file.mtime)}</td><td class="cell-size">{if file.path_type.ends_with("Dir"){String::new()}else{model::size(file.size)}}</td></tr>}
}
#[component]
pub fn Icon(name: &'static str) -> impl IntoView {
    let path = match name {
        "upload" => "M12 16V3m-5 5 5-5 5 5M4 15v6h16v-6",
        "folder" => "M3 7V4h6l3 3h9v14H3V7",
        "home" => "m3 10 9-8 9 8M5 8v13h14V8M9 21v-8h6v8",
        "move" => "M4 4v7a5 5 0 0 0 5 5h11m-5-5 5 5-5 5",
        "download" => "M12 3v13m-5-5 5 5 5-5M4 18v3h16v-3",
        "delete" => "M4 6h16M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7m4-7v7",
        "rename" => "m4 17-1 4 4-1L21 6l-4-4L4 17m10-12 4 4",
        "tags" => "M3 3h8l10 10-8 8L3 11V3m4 4h.01",
        "logout" => "M10 3H3v18h7m5-15 6 6-6 6m-8-6h14",
        "refresh" => "M20 7v5h-5M4 17a9 9 0 1 0 1-12",
        "account" => "M20 21a8 8 0 0 0-16 0M12 3a4 4 0 1 0 0 8 4 4 0 0 0 0-8",
        "create" => "M12 3v18M3 12h18",
        _ => "M5 3h9l5 5v13H5V3m9 0v6h5",
    };
    view! {<svg aria-hidden="true" width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d=path/></svg>}
}

fn window_move(model: Model, next: bool) {
    model.listing.update(|l| {
        l.window_start = if next {
            (l.window_start + 200).min(l.files.len().saturating_sub(200))
        } else {
            l.window_start.saturating_sub(200)
        }
    });
    let l = model.listing.get_untracked();
    let selector = if !next && l.window_start == 0
        || next && l.window_start + 200 >= l.files.len() && l.next.is_none()
    {
        ".list-status"
    } else if next && l.window_start + 200 >= l.files.len() {
        ".list-pagination > .load-more:last-child"
    } else if next {
        ".list-pagination > .load-more:nth-of-type(2)"
    } else {
        ".list-pagination > .load-more:first-of-type"
    };
    gloo_timers::callback::Timeout::new(0, move || {
        if let Some(el) = http::document()
            .query_selector(selector)
            .ok()
            .flatten()
            .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let _ = el.focus();
        }
    })
    .forget();
}
