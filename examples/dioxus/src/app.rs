//! The page, its styles and the brand assets, and the browser actions a home-screen web app
//! needs.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use dioxus::prelude::*;
use futures_util::StreamExt;
use pushups_example_support as support;

use super::log;

/// The platforms where a denied permission lives in the system settings the example opens.
const OPEN_SETTINGS: bool = cfg!(any(
    target_os = "ios",
    target_os = "android",
    target_os = "macos",
    target_os = "windows"
));

#[component]
pub fn App() -> Element {
    let mut lines = use_signal(Vec::<support::Line>::new);
    let mut permission = use_signal(|| None::<support::PermissionUi>);
    let mut registration = use_signal(|| None::<pushups::Token>);
    let mut settings_error = use_signal(|| None::<String>);
    let favicon = use_hook(|| format!("data:image/svg+xml;base64,{}", STANDARD.encode(FAVICON)));
    use_future(move || async move {
        log(&format!("ui mounted at_ms={}", support::now_ms()));
        let mut events = pushups::events();
        if let Err(error) = pushups::register() {
            lines
                .write()
                .push(support::Line::Text(format!("register failed: {error}")));
        }
        while let Some(event) = events.next().await {
            log(&support::describe(&event));
            if let pushups::Event::Token(token) = &event {
                registration.write().replace(token.clone());
            }
            lines.write().push(support::line(&event, support::now_ms()));
            if let Err(error) = support::register_again(&event) {
                lines
                    .write()
                    .push(support::Line::Text(format!("register failed: {error}")));
            }
        }
    });
    // The permission the platform holds, re-read when the page or webview comes back to the
    // foreground, where the user may have changed it in Settings.
    let mut read = permission;
    use_future(move || async move {
        let mut eval = document::eval(FOREGROUND);
        loop {
            let status = pushups::permission_status().await;
            log(&format!(
                "permission {}",
                support::permission(status.clone())
            ));
            read.set(Some(support::permission_ui(status)));
            // The foreground script stops sending once the page is gone.
            if eval.recv::<bool>().await.is_err() {
                break;
            }
        }
    });
    let entries: Vec<Element> = lines.read().iter().rev().map(entry_view).collect();
    rsx! {
        document::Style { {support::page::CSS} }
        document::Link { rel: "icon", href: favicon }
        div { class: "app",
            div { class: "header",
                div { class: "mark", dangerous_inner_html: support::page::MARK }
                h2 { "pushups example" }
            }
            div { class: "status",
                div { class: "row",
                    span { class: "label", "Transport" }
                    span { class: "muted", {registration.read().as_ref().map_or("—", support::transport)} }
                }
                div { class: "row",
                    span { class: "muted", {permission.read().as_ref().map(|ui| ui.label.clone()).unwrap_or_default()} }
                    if permission.read().as_ref().is_some_and(|ui| ui.asks) {
                        button {
                            onclick: move |_| async move {
                                let status = pushups::request_permission().await;
                                let answer = support::permission(status.clone());
                                log(&format!("permission {answer}"));
                                permission.set(Some(support::permission_ui(status)));
                            },
                            "Allow notifications"
                        }
                    }
                    if OPEN_SETTINGS && permission.read().as_ref().is_some_and(|ui| ui.denied) {
                        button {
                            class: "secondary",
                            onclick: move |_| {
                                match pushups::open_notification_settings() {
                                    Ok(()) => *settings_error.write() = None,
                                    Err(error) => {
                                        *settings_error.write() = Some(format!("{error}"))
                                    }
                                }
                            },
                            "Open Settings"
                        }
                    }
                }
                if registration.read().is_some() {
                    div { class: "row",
                        span { class: "token", {registration.read().as_ref().map_or_default(support::shortened)} }
                        button {
                            class: "copy",
                            onclick: move |_| {
                                if let Some(token) = registration.read().clone() {
                                    copy_text(&support::token(&token));
                                }
                            },
                            "Copy"
                        }
                    }
                }
                if let Some(error) = settings_error.read().clone() {
                    p { class: "error", {error} }
                }
            }
            div { class: "actions",
                if cfg!(target_arch = "wasm32") {
                    button { onclick: move |_| reload_page(), "Reload" }
                }
                button {
                    onclick: move |_| copy_text(&support::log_text(&lines.read())),
                    "Copy log"
                }
            }
            {entries.into_iter()}
        }
    }
}

/// The line of the list the page shows, a push card or a compact event.
fn entry_view(entry: &support::Line) -> Element {
    match entry {
        support::Line::Push(push) => rsx! {
            div { class: "card",
                p { class: "title", {push.title.clone()} }
                p { class: "body", {push.body.clone()} }
                div { class: "meta",
                    span { {push.time.clone()} }
                    if push.started_app { span { class: "badge", "started the app" } }
                    if let Some(delay) = push.delay_ms { span { {format!("+{delay} ms")} } }
                }
            }
        },
        support::Line::Text(text) => rsx! {
            p { class: "line", {text.clone()} }
        },
    }
}

/// The page or webview is back in the foreground, so the permission read re-runs.
const FOREGROUND: &str = "
document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') dioxus.send(true);
});
window.addEventListener('focus', () => dioxus.send(true));
";

/// The browser tab's icon, linked as a `data:` URL since `public_dir` serves only the crate's `js/`.
const FAVICON: &[u8] = include_bytes!("../../../assets/brand/favicon.svg");

/// Reloads the page, the only way to restart a home-screen web app on iOS from inside it.
#[cfg(target_arch = "wasm32")]
fn reload_page() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let _ = window.location().reload();
}

#[cfg(not(target_arch = "wasm32"))]
fn reload_page() {}

/// Puts `text` on the clipboard, so a phone can paste the log into a message, through the
/// webview's clipboard API with a fallback to a hidden field.
fn copy_text(text: &str) {
    let script = support::page::clipboard_script(text);
    document::eval(&script);
}
