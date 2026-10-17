use xcss_web::component::{component_props_builder, component_view};
mod http;
mod model;
mod operations;
mod protocol;
mod tags;
mod upload;
mod workspace;
use wasm_bindgen::{JsCast, prelude::*};
use xcss_web::i18n::t;
use xcss_web::prelude::*;

#[wasm_bindgen(start)]
pub fn start() {
    if let Some(root) = http::document().get_element_by_id("xczs-root") {
        let _ = root.set_attribute("data-xczs-renderer", "xcss-web");
    }
    xcss_web::mount("xczs-root", App);
}
#[component]
fn App() -> impl IntoView {
    let loading = RwSignal::new(true);
    let failure = RwSignal::new(String::new());
    let ready = RwSignal::new(None::<model::Model>);
    let is_login = http::window().location().pathname().unwrap_or_default() == "/__xczs__/login";
    if !is_login {
        xcss_web::task::spawn_local(async move {
            let result = async {
                let session = xcss_web::api::restore().await.map_err(|e| e.to_string())?;
                let _ = session;
                let template = http::document()
                    .get_element_by_id("index-data")
                    .and_then(|e| e.dyn_into::<web_sys::HtmlTemplateElement>().ok())
                    .ok_or_else(|| t("页面数据缺失", "Page data is missing"))?;
                let text = template.content().text_content().unwrap_or_default();
                let binary = http::window()
                    .atob(text.trim())
                    .map_err(|_| t("嵌入页面数据无效", "Invalid embedded index data"))?;
                let bytes = binary.chars().map(|c| c as u8).collect::<Vec<_>>();
                let value: serde_json::Value = serde_json::from_slice(&bytes)
                    .map_err(|_| t("嵌入页面数据无效", "Invalid embedded index data"))?;
                let (directory, exists) = index_metadata(&value)?;
                Ok(model::Model::new(directory, exists))
            }
            .await;
            match result {
                Ok(model) => ready.set(Some(model)),
                Err(e) => {
                    failure.set(e);
                    if xcss_web::api::current_session().is_none() {
                        let _ = http::window().location().replace("/__xczs__/login");
                    }
                }
            }
            loading.set(false);
        });
    }
    if is_login {
        View::new({
            #[allow(unreachable_code)]
            #[allow(unused_mut)]
            #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
            component_view(
                #[allow(clippy::needless_borrows_for_generic_args)]
                &Login,
                {
                    let mut props = component_props_builder(&Login).build();
                    props
                },
            )
        })
        .into_any()
    } else {
        View::new((
            {
                #[allow(unreachable_code)]
                #[allow(unused_mut)]
                #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
                component_view(
                    #[allow(clippy::needless_borrows_for_generic_args)]
                    &Show,
                    {
                        let mut props = component_props_builder(&Show)
                            .when(
                                #[allow(unused_braces)]
                                {
                                    move || loading.get()
                                },
                            )
                            .children({
                                ::xcss_web::children::ToChildren::to_children(move || {
                                    xcss_web::html::p()
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
            xcss_web::html::p()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({ move || failure.get() })
                    },
                )
                .class("workspace-status")
                .role("alert")
                .hidden(
                    ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move || {
                        failure.get().is_empty()
                    }),
                ),
            IntoRender::into_render({
                move || {
                    ready.get().map(|model| {
                        View::new({
                            #[allow(unreachable_code)]
                            #[allow(unused_mut)]
                            #[allow(
                                clippy::let_and_return,
                                clippy::let_unit_value,
                                clippy::unit_arg
                            )]
                            component_view(
                                #[allow(clippy::needless_borrows_for_generic_args)]
                                &workspace::Workspace,
                                {
                                    let mut props = component_props_builder(&workspace::Workspace)
                                        .model(
                                            #[allow(unused_braces)]
                                            {
                                                model
                                            },
                                        )
                                        .build();
                                    props
                                },
                            )
                        })
                    })
                }
            }),
        ))
        .into_any()
    }
}
#[component]
fn Login() -> impl IntoView {
    http::document().set_title(&t("登录", "Sign in"));
    let username = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let pending = RwSignal::new(false);
    let error = RwSignal::new(String::new());
    let field = RwSignal::new(String::new());
    let root = http::document().get_element_by_id("xczs-root").unwrap();
    let min = root
        .get_attribute("data-min-bytes")
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(12);
    let max = root
        .get_attribute("data-max-bytes")
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(1024);
    View::new(
    xcss_web::html::main()
        .child((
            xcss_web::html::div()
                .child(
                    #[allow(unused_braces)]
                    {
                        xcss_web::html::div()
                            .child((
                                {
                                    #[allow(unreachable_code)] #[allow(unused_mut)]
                                    #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
                                    component_view(
                                        #[allow(clippy::needless_borrows_for_generic_args)]
                                        &xcss_web::components::LanguageToggle,
                                        {
                                            let mut props = component_props_builder(
                                                    &xcss_web::components::LanguageToggle,
                                                )
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
                                        &xcss_web::components::ThemeToggle,
                                        {
                                            let mut props = component_props_builder(
                                                    &xcss_web::components::ThemeToggle,
                                                )
                                                .build();
                                            props
                                        },
                                    )
                                },
                            ))
                            .class("xcss-login-controls")
                            .role("group")
                            .aria_label(
                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                                    t("显示设置", "Display settings"),
                                ),
                            )
                    },
                )
                .class("xcss-auth-language")
                .style(("position", "absolute"))
                .style(("inset-block-start", "1rem"))
                .style(("inset-inline-end", "1rem")),
            xcss_web::html::section()
                .child(
                    #[allow(unused_braces)]
                    {
                        xcss_web::html::form()
                            .child((
                                xcss_web::html::h1()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({
                                                t("登录", "Sign in")
                                            })
                                        },
                                    )
                                    .class("visually-hidden"),
                                xcss_web::html::label()
                                    .child((
                                        xcss_web::html::span()
                                            .child(
                                                #[allow(unused_braces)]
                                                {
                                                    IntoRender::into_render({
                                                        t("用户名", "Username")
                                                    })
                                                },
                                            ),
                                        xcss_web::html::input()
                                            .class("xcss-input login-input")
                                            .id("username")
                                            .name("username")
                                            .r#type("text")
                                            .maxlength("64")
                                            .autocomplete("username")
                                            .autocapitalize("none")
                                            .spellcheck("false")
                                            .required(true)
                                            .autofocus(true)
                                            .prop("value", move || username.get())
                                            .readonly(
                                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                                pending.get()),
                                            )
                                            .aria_invalid(
                                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                                (field.get() == "username").then_some("true")),
                                            )
                                            .aria_describedby(
                                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                                (!error.get().is_empty()).then_some("login-error")),
                                            )
                                            .on(
                                                xcss_web::ev::input,
                                                move |e| username.set(event_target_value(&e)),
                                            ),
                                    ))
                                    .class("xcss-form-field")
                                    .r#for("username"),
                                xcss_web::html::label()
                                    .child((
                                        xcss_web::html::span()
                                            .child(
                                                #[allow(unused_braces)]
                                                {
                                                    IntoRender::into_render({
                                                        t("密码", "Password")
                                                    })
                                                },
                                            ),
                                        xcss_web::html::input()
                                            .class("xcss-input login-input")
                                            .id("password")
                                            .name("password")
                                            .r#type("password")
                                            .attr(
                                                "data-min-bytes",
                                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                                                    min,
                                                ),
                                            )
                                            .attr(
                                                "data-max-bytes",
                                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                                                    max,
                                                ),
                                            )
                                            .autocomplete("current-password")
                                            .required(true)
                                            .prop("value", move || password.get())
                                            .readonly(
                                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                                pending.get()),
                                            )
                                            .aria_invalid(
                                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                                (field.get() == "password").then_some("true")),
                                            )
                                            .aria_describedby(
                                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                                (!error.get().is_empty()).then_some("login-error")),
                                            )
                                            .on(
                                                xcss_web::ev::input,
                                                move |e| password.set(event_target_value(&e)),
                                            ),
                                    ))
                                    .class("xcss-form-field")
                                    .r#for("password"),
                                xcss_web::html::div()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            xcss_web::html::span()
                                                .child(
                                                    #[allow(unused_braces)]
                                                    {
                                                        IntoRender::into_render({
                                                            move || error.get()
                                                        })
                                                    },
                                                )
                                                .class("login-error")
                                                .id("login-error")
                                        },
                                    )
                                    .class("xcss-error error-row")
                                    .class(("hidden", move || error.get().is_empty()))
                                    .role("alert"),
                                xcss_web::html::button()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({
                                                t("登录", "Sign in")
                                            })
                                        },
                                    )
                                    .class("xcss-button")
                                    .r#type("submit")
                                    .disabled(
                                        ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                        pending.get()),
                                    ),
                            ))
                            .class("login-card")
                            .aria_label(
                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                                    t("登录 xczs", "Sign in to xczs"),
                                ),
                            )
                            .aria_busy(
                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                pending.get().to_string()),
                            )
                            .novalidate(true)
                            .on(
                                xcss_web::ev::input,
                                move |_| {
                                    field.set(String::new());
                                    error.set(String::new());
                                },
                            )
                            .on(
                                xcss_web::ev::submit,
                                move |e| {
                                    e.prevent_default();
                                    if pending.get_untracked() {
                                        return;
                                    }
                                    let user = username.get_untracked();
                                    let pass = password.get_untracked();
                                    if user.is_empty() || pass.is_empty() {
                                        let f = if user.is_empty() {
                                            "username"
                                        } else {
                                            "password"
                                        };
                                        field.set(f.into());
                                        error
                                            .set(
                                                if f == "username" {
                                                    t("请输入用户名。", "Enter your username.")
                                                } else {
                                                    t("请输入密码。", "Enter your password.")
                                                },
                                            );
                                        focus(f);
                                        return;
                                    }
                                    if !(min..=max).contains(&pass.len())
                                        || !xcss_web::contracts::valid_password(&pass)
                                        || !xcss_web::contracts::valid_username_candidate(&user)
                                    {
                                        password.set(String::new());
                                        field.set("password".into());
                                        error
                                            .set(
                                                t(
                                                    "请输入有效的管理员用户名和密码。",
                                                    "Enter a valid administrator username and password.",
                                                ),
                                            );
                                        focus("password");
                                        return;
                                    }
                                    pending.set(true);
                                    error.set(String::new());
                                    xcss_web::task::spawn_local(async move {
                                        match xcss_web::api::login(&user, &pass).await {
                                            Ok(_) => {
                                                let _ = http::window()
                                                    .location()
                                                    .set_href(&login_landing());
                                            }
                                            Err(e) => {
                                                error
                                                    .set(
                                                        format!(
                                                            "{}{}", t("无法完成认证请求，请重试。",
                                                            "Authentication request could not be completed. Please try again."),
                                                            e.request_id.map(| id | format!(" Request ID: {id}"))
                                                            .unwrap_or_default()
                                                        ),
                                                    );
                                                field.set(String::new());
                                            }
                                        }
                                        password.set(String::new());
                                        pending.set(false);
                                        focus("password");
                                    });
                                },
                            )
                    },
                )
                .class("xcss-auth-card")
                .aria_label("xczs"),
        ))
        .class("xcss-auth-shell login-screen"),
)
}
fn focus(id: &str) {
    if let Some(e) = http::document()
        .get_element_by_id(id)
        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = e.focus();
    }
}

#[wasm_bindgen]
pub fn administrator_session_json() -> String {
    serde_json::to_string(&xcss_web::api::current_session()).unwrap_or_else(|_| "null".into())
}

/// Read-only job status API shared with mutation reconciliation.
#[wasm_bindgen]
pub async fn query_job_json(id: String) -> String {
    http::query_job(&id).await.to_string()
}

fn login_landing() -> String {
    let current = http::window().location().href().unwrap_or_default();
    if let Ok(url) = web_sys::Url::new(&current)
        && url.search_params().has("lang")
    {
        return format!("/?lang={}", xcss_web::i18n::locale());
    }
    "/".into()
}

fn index_metadata(value: &serde_json::Value) -> Result<(String, bool), String> {
    let invalid = || t("嵌入页面数据无效", "Invalid embedded index data");
    let object = value
        .as_object()
        .filter(|object| object.len() == 2)
        .ok_or_else(invalid)?;
    let directory = object
        .get("href")
        .and_then(|value| value.as_str())
        .ok_or_else(invalid)?;
    let exists = object
        .get("dir_exists")
        .and_then(|value| value.as_bool())
        .ok_or_else(invalid)?;
    if directory != "/" && (!directory.starts_with('/') || !http::valid_path(&directory[1..])) {
        return Err(t("嵌入页面路径无效", "Invalid embedded page path"));
    }
    Ok((directory.into(), exists))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn index_metadata_admits_only_two_business_fields_and_canonical_absolute_paths() {
        for href in [
            "/",
            "/folder/文件 & name",
            "/目录/with space/100% literal",
            r"/back\slash/is-valid-on-linux",
        ] {
            assert_eq!(
                index_metadata(&json!({"href":href,"dir_exists":true})),
                Ok((href.into(), true))
            );
        }
        for value in [
            json!(null),
            json!([]),
            json!({"href":"/"}),
            json!({"dir_exists":true}),
            json!({"href":"/","dir_exists":true,"session":{"csrf_token":"forbidden"}}),
            json!({"href":"/","dir_exists":true,"extra":true}),
        ] {
            assert!(index_metadata(&value).is_err());
        }
        for href in [
            "",
            "relative",
            "./relative",
            "../relative",
            "//",
            "//folder",
            "/folder/",
            "/folder//child",
            "/./folder",
            "/folder/.",
            "/../folder",
            "/folder/..",
            "/nul\0byte",
        ] {
            assert!(
                index_metadata(&json!({"href":href,"dir_exists":true})).is_err(),
                "{href:?}"
            );
        }
        for flag in [json!(0), json!(1), json!("true"), json!(null)] {
            assert!(index_metadata(&json!({"href":"/","dir_exists":flag})).is_err());
        }
    }
}
