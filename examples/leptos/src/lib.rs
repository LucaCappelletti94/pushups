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
    use pushups::{Config, Message, Notification};

    use super::{log, support};

    pub(super) fn config() -> Config {
        support::web_push(Some("/sw.js")).map_or_else(Config::new, |web| Config::new().web(web))
    }

    /// Runs in the service worker for every push, and decides what the user sees.
    pub(super) async fn notification_for(message: Message) -> Notification {
        support::worker_notification(&message, log)
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
    let permission = RwSignal::new(None::<support::PermissionUi>);
    // Effects run only in the browser, after hydration.
    Effect::new(move |_| {
        spawn_local_scoped_with_cancellation(async move {
            log(&format!("ui mounted at_ms={}", support::now_ms()));
            // The permission the platform holds, read without asking, so the page shows it.
            let status = pushups::permission_status().await;
            log(&format!(
                "permission {}",
                support::permission(status.clone())
            ));
            permission.set(Some(support::permission_ui(status)));
            let mut events = pushups::events();
            if let Err(error) = pushups::register() {
                lines.update(|lines| lines.push(format!("register failed: {error}")));
            }
            while let Some(event) = events.next().await {
                log(&support::describe(&event));
                lines.update(|lines| lines.push(support::shown(&event)));
            }
        });
    });
    // The user can change the permission in Settings, so the page re-reads it when it comes
    // back to the foreground.
    let on_foreground = permission.clone();
    let handle = window_event_listener_untyped("visibilitychange", move |_| {
        if leptos::web_sys::window()
            .and_then(|window| window.document())
            .is_some_and(|document| {
                document.visibility_state() == leptos::web_sys::VisibilityState::Visible
            })
        {
            let on_foreground = on_foreground.clone();
            leptos::task::spawn_local(async move {
                let status = pushups::permission_status().await;
                let answer = support::permission(status.clone());
                log(&format!("permission {answer}"));
                on_foreground.set(Some(support::permission_ui(status)));
            });
        }
    });
    on_cleanup(move || handle.remove());
    // The ask button exists only while asking is offered, and the state shows on its own line.
    view! {
        <div style="font-family: system-ui, sans-serif; color: #f5f7fa; padding: 20px 16px;">
            <h2>"pushups Leptos example"</h2>
            {move || {
                let permission = permission.clone();
                permission.get().is_some_and(|ui| ui.asks).then(|| {
                    view! {
                        <button on:click=move |_| {
                            let permission = permission.clone();
                            spawn_local_scoped_with_cancellation(async move {
                                let status = pushups::request_permission().await;
                                let answer = support::permission(status.clone());
                                log(&format!("permission {answer}"));
                                permission.set(Some(support::permission_ui(status)));
                            });
                        }>"Allow notifications"</button>
                    }
                })
            }}
            <p>{move || permission.get().map(|ui| ui.label.clone()).unwrap_or_default()}</p>
            <For
                each=move || lines.get().into_iter().enumerate()
                key=|(index, _)| *index
                children=|(_, line)| view! { <p style="font-family: monospace; font-size: 12px;">{line}</p> }
            />
        </div>
    }
}

fn log(line: &str) {
    support::log("pushups-example", line);
}
