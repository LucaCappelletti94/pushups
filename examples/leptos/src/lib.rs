//! A Leptos app rendered by its server and hydrated in the browser, receiving pushes through
//! `pushups` and logging every step to the console with its wall-clock time, so the CI check
//! can time send to delivery.
//!
//! `PUSHUPS_VAPID_PUBLIC_KEY` (base64url) at build time sets the app server's VAPID key.
//! `PUSHUPS_WEB_WORKER=static` registers the static worker the server serves from
//! `pushups::SERVICE_WORKER`, in place of the Rust handler, which runs from the `sw.js` entry.

use futures_util::StreamExt;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::StaticSegment;
use leptos_router::components::{Route, Router, Routes};
use pushups::{Event, Message, Permission};

/// The worker entry the server serves at `/sw.js` under `rust_handler`.
pub const WORKER_ENTRY: &str = include_str!("../sw.js");

/// Starts the page's wasm, from Leptos's hydration script.
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    match pushups::install(client::config()) {
        Ok(()) => log(&format!("installed at_ms={}", now_ms())),
        Err(error) => log(&format!("install failed: {error}")),
    }
    leptos::mount::hydrate_body(App);
}

/// Runs whenever the wasm starts, so in the worker the `sw.js` entry starts it serves the handler.
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    if pushups::in_service_worker()
        && let Err(error) = pushups::serve_service_worker(client::notification_for)
    {
        log(&format!("serving the service worker failed: {error}"));
    }
}

#[cfg(feature = "hydrate")]
mod client {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use pushups::{Config, Message, Notification, WebPushConfig};

    use super::{log, now_ms, timing};

    pub(super) fn config() -> Config {
        let key = option_env!("PUSHUPS_VAPID_PUBLIC_KEY")
            .and_then(|key| URL_SAFE_NO_PAD.decode(key).ok())
            .and_then(|key| <[u8; 65]>::try_from(key).ok());
        let Some(key) = key else {
            return Config::new();
        };
        let web = WebPushConfig::new(key);
        Config::new().web(match option_env!("PUSHUPS_WEB_WORKER") {
            Some("static") => web,
            _ => web.rust_handler().service_worker_path("/sw.js"),
        })
    }

    /// Runs in the service worker for every push, and decides what the user sees.
    pub(super) async fn notification_for(message: Message) -> Notification {
        let at = now_ms();
        let line = timing(&message, at);
        log(&format!("worker push at_ms={at} {line}"));
        Notification::new("pushups from Rust")
            .body(format!("built in the service worker, {line}"))
            .navigate("/")
    }
}

/// The page around the app, rendered by the server.
pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <title>"pushups Leptos example"</title>
                <HydrationScripts options/>
            </head>
            <body style="margin: 0; background: #0e1824;">
                <App/>
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    view! {
        <Router>
            <Routes fallback=|| "Page not found.">
                <Route path=StaticSegment("") view=Home/>
            </Routes>
        </Router>
    }
}

#[component]
fn Home() -> impl IntoView {
    let lines = RwSignal::new(Vec::<String>::new());
    let permission = RwSignal::new(String::new());
    // Effects run only in the browser, after hydration.
    Effect::new(move |_| {
        spawn_local(async move {
            log(&format!("ui mounted at_ms={}", now_ms()));
            let mut events = pushups::events();
            if let Err(error) = pushups::register() {
                lines.update(|lines| lines.push(format!("register failed: {error}")));
            }
            while let Some(event) = events.next().await {
                let line = describe(&event);
                log(&line);
                lines.update(|lines| lines.push(line));
            }
        });
    });
    let allow = move |_| {
        spawn_local(async move {
            let answer = match pushups::request_permission().await {
                Ok(Permission::Granted) => {
                    // Safari subscribes only once the user allowed notifications, so the token comes now.
                    if let Err(error) = pushups::register() {
                        log(&format!("register failed: {error}"));
                    }
                    "granted".to_owned()
                }
                Ok(Permission::Denied) => "denied".to_owned(),
                Ok(other) => format!("{other:?}"),
                Err(error) => format!("failed: {error}"),
            };
            log(&format!("permission {answer}"));
            permission.set(answer);
        });
    };
    view! {
        <div style="font-family: system-ui, sans-serif; color: #f5f7fa; padding: 20px 16px;">
            <h2>"pushups Leptos example"</h2>
            <button on:click=allow>"Allow notifications"</button>
            <p>"permission: " {move || permission.get()}</p>
            <For
                each=move || lines.get().into_iter().enumerate()
                key=|(index, _)| *index
                children=|(_, line)| view! { <p style="font-family: monospace; font-size: 12px;">{line}</p> }
            />
        </div>
    }
}

fn describe(event: &Event) -> String {
    let at = now_ms();
    match event {
        Event::Token(token @ pushups::Token::WebPush { endpoint, .. }) => {
            let keys = token.web_push_keys().expect("a Web Push token has keys");
            let subscription = serde_json::json!({
                "endpoint": endpoint,
                "keys": { "p256dh": keys.p256dh, "auth": keys.auth },
            });
            format!("ui token at_ms={at} subscription={subscription}")
        }
        Event::Token(token) => format!("ui token at_ms={at} {token:?}"),
        Event::RegistrationFailed(error) => format!("ui registration failed at_ms={at} {error}"),
        Event::Message(message) => format!(
            "ui message at_ms={at} started_app={} {}",
            message.started_app,
            timing(message, at)
        ),
        Event::MessagesDropped => format!("ui messages dropped at_ms={at}"),
    }
}

/// The push's `seq` and its latency from the sender's `sent_at_ms` to `at`.
fn timing(message: &Message, at: u128) -> String {
    let data: serde_json::Value = serde_json::from_slice(&message.payload).unwrap_or_default();
    let field = |name: &str| data.get(name).and_then(serde_json::Value::as_str);
    let sent = field("sent_at_ms").and_then(|sent| sent.parse::<u128>().ok());
    format!(
        "seq={} sent_at_ms={} since_sent_ms={}",
        field("seq").unwrap_or("-"),
        sent.map_or_else(|| "-".to_owned(), |sent| sent.to_string()),
        sent.map_or_else(
            || "-".to_owned(),
            |sent| at.saturating_sub(sent).to_string()
        ),
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis())
}

/// `SystemTime` has no clock on `wasm32-unknown-unknown`.
#[cfg(target_arch = "wasm32")]
fn now_ms() -> u128 {
    let ms = js_sys::Date::now();
    // Milliseconds since 1970 are positive and far below 2^53, so the conversion is exact.
    debug_assert!(ms.is_finite() && ms >= 0.0, "Date.now() is a time");
    ms as u128
}

#[cfg(target_arch = "wasm32")]
fn log(line: &str) {
    web_sys::console::log_1(&format!("pushups-example {line}").into());
}

#[cfg(not(target_arch = "wasm32"))]
fn log(line: &str) {
    println!("pushups-example {line}");
}
