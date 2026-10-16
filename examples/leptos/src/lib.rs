//! A Leptos app rendered by its server and hydrated in the browser, receiving pushes through
//! `pushups` and logging every step to the console with its wall-clock time, so the CI check
//! can time send to delivery.
//!
//! `PUSHUPS_VAPID_PUBLIC_KEY` (base64url) at build time sets the app server's VAPID key.
//! `PUSHUPS_WEB_WORKER=static` registers the static worker the server serves from
//! `pushups::SERVICE_WORKER`, in place of the Rust handler, which runs from the `sw.js` entry.

use futures_util::StreamExt;
use leptos::prelude::*;
use leptos::task::spawn_local_scoped_with_cancellation;
use leptos_router::StaticSegment;
use leptos_router::components::{Route, Router, Routes};
use pushups::{Event, Permission};
use pushups_example_support as support;

/// The worker entry the server serves at `/sw.js` under `rust_handler`.
pub const WORKER_ENTRY: &str = include_str!("../sw.js");

/// Starts the page's wasm, from Leptos's hydration script.
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    match pushups::install(client::config()) {
        Ok(()) => log(&format!("installed at_ms={}", support::now_ms())),
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
    use pushups::{Config, Message, Notification, VapidPublicKey, WebPushConfig};

    use super::{log, support};

    pub(super) fn config() -> Config {
        let Some(key) = option_env!("PUSHUPS_VAPID_PUBLIC_KEY") else {
            return Config::new();
        };
        let key =
            VapidPublicKey::from_base64url(key).expect("PUSHUPS_VAPID_PUBLIC_KEY is a VAPID key");
        let web = WebPushConfig::new(key);
        Config::new().web(match option_env!("PUSHUPS_WEB_WORKER") {
            Some("static") => web,
            _ => web.rust_handler().service_worker_path("/sw.js"),
        })
    }

    /// Runs in the service worker for every push, and decides what the user sees.
    pub(super) async fn notification_for(message: Message) -> Notification {
        let at = support::now_ms();
        let line = support::timing(&message, at);
        log(&format!("worker push at_ms={at} {line}"));
        Notification::new(
            support::field(&message, "title").unwrap_or_else(|| "pushups from Rust".to_owned()),
        )
        .body(
            support::field(&message, "body")
                .unwrap_or_else(|| format!("built in the service worker, {line}")),
        )
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
        spawn_local_scoped_with_cancellation(async move {
            log(&format!("ui mounted at_ms={}", support::now_ms()));
            let mut events = pushups::events();
            if let Err(error) = pushups::register() {
                lines.update(|lines| lines.push(format!("register failed: {error}")));
            }
            while let Some(event) = events.next().await {
                log(&describe(&event));
                lines.update(|lines| lines.push(support::shown(&event)));
            }
        });
    });
    let allow = move |_| {
        spawn_local_scoped_with_cancellation(async move {
            let answer = match pushups::request_permission().await {
                Ok(Permission::Granted) => "granted".to_owned(),
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

fn log(line: &str) {
    support::log("pushups-example", line);
}
