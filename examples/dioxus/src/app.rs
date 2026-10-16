//! The page, its styles and the brand assets, and the browser actions a home-screen web app
//! needs.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use dioxus::prelude::*;
use futures_util::StreamExt;
use pushups::{Event, Permission};
use pushups_example_support as support;

use super::log;

#[component]
pub fn App() -> Element {
    let mut lines = use_signal(Vec::<String>::new);
    let mut permission = use_signal(|| None::<String>);
    let favicon = use_hook(|| format!("data:image/svg+xml;base64,{}", STANDARD.encode(FAVICON)));
    use_future(move || async move {
        log(&format!("ui mounted at_ms={}", support::now_ms()));
        let mut events = pushups::events();
        if let Err(error) = pushups::register() {
            lines.write().push(format!("register failed: {error}"));
        }
        while let Some(event) = events.next().await {
            log(&describe(&event));
            lines.write().push(support::shown(&event));
        }
    });
    rsx! {
        document::Style { {PAGE} }
        document::Link { rel: "icon", href: favicon }
        div { class: "app",
            div { class: "header",
                div { class: "mark", dangerous_inner_html: MARK }
                h2 { "pushups example" }
            }
            div { class: "actions",
                button {
                    onclick: move |_| async move {
                        let answer = match pushups::request_permission().await {
                            Ok(Permission::Granted) => "granted".to_owned(),
                            Ok(Permission::Denied) => "denied".to_owned(),
                            Ok(other) => format!("{other:?}"),
                            Err(error) => format!("failed: {error}"),
                        };
                        log(&format!("permission {answer}"));
                        permission.set(Some(answer));
                    },
                    "Allow notifications"
                }
                if cfg!(target_arch = "wasm32") {
                    button { onclick: move |_| reload_page(), "Reload" }
                    button {
                        onclick: move |_| copy_text(&lines.read().join("\n")),
                        "Copy log"
                    }
                }
            }
            p { class: "muted", "permission: " {permission.read().clone().unwrap_or_default()} }
            for line in lines.read().iter() {
                p { class: "line", {line.clone()} }
            }
        }
    }
}

/// The page's styles in the brand's colours from `assets/brand`, a navy ground and the mark's red-to-orange gradient.
const PAGE: &str = "
:root { --navy: #0e1824; --surface: #172436; --text: #f5f7fa; --muted: #9aa6b8; --gradient: linear-gradient(135deg, #fa1e13, #f67017); }
html, body { margin: 0; min-height: 100%; background: var(--navy); }
.app { font-family: -apple-system, system-ui, sans-serif; color: var(--text); padding: 20px 16px; max-width: 640px; margin: 0 auto; box-sizing: border-box; }
.header { display: flex; align-items: center; gap: 12px; margin-bottom: 16px; }
.header h2 { margin: 0; font-size: 26px; font-weight: 700; }
.mark { width: 56px; flex: none; }
.mark svg { display: block; width: 100%; height: auto; }
.actions { display: flex; gap: 8px; flex-wrap: wrap; }
.actions button { font-size: 16px; font-weight: 600; color: #ffffff; padding: 10px 16px; border: none; border-radius: 10px; background: var(--gradient); }
.muted { color: var(--muted); }
.line { font-family: ui-monospace, monospace; font-size: 12px; overflow-wrap: anywhere; background: var(--surface); padding: 8px 10px; border-radius: 8px; margin: 8px 0; }
";

/// The brand mark, drawn inline so every platform shows it without an asset pipeline.
const MARK: &str = include_str!("../../../assets/brand/mark.svg");

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

/// Puts `text` on the clipboard, so a phone can paste the log into a message.
#[cfg(target_arch = "wasm32")]
fn copy_text(text: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let clipboard = window.navigator().clipboard();
    let _ = clipboard.write_text(text);
}

#[cfg(not(target_arch = "wasm32"))]
fn copy_text(_text: &str) {}

fn describe(event: &Event) -> String {
    let at = support::now_ms();
    match event {
        Event::Token(token) => format!("ui token at_ms={at} {}", support::token(token)),
        Event::RegistrationFailed(error) => format!("ui registration failed at_ms={at} {error}"),
        Event::Message(message) => format!(
            "ui message at_ms={at} started_app={} {}",
            message.started_app,
            support::timing(message, at)
        ),
        Event::MessagesDropped => format!("ui messages dropped at_ms={at}"),
    }
}
