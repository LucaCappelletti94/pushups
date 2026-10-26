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
    if pushups::in_service_worker() {
        if let Err(error) = pushups::serve_service_worker(client::notification_for) {
            log(&format!("serving the service worker failed: {error}"));
        }
    }
}

#[cfg(feature = "hydrate")]
mod client {
    use pushups::{Config, Message, Notification};
    use pushups_example_support as support;

    pub fn config() -> Config {
        support::web_push(Some("/sw.js")).map_or_else(Config::new, |web| Config::new().web(web))
    }

    /// Runs in the service worker for every push, and decides what the user sees.
    pub async fn notification_for(message: Message) -> Notification {
        support::worker_notification(&message, log)
    }

    fn log(line: &str) {
        super::log(line);
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
                <style>{ support::page::CSS }</style>
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

/// The line of the list the page shows, a push card or a compact event.
fn entry_view(entry: &support::Line) -> AnyView {
    match entry {
        support::Line::Push(push) => {
            view! {
                <div class="card">
                    <p class="title">{ push.title.clone() }</p>
                    <p class="body">{ push.body.clone() }</p>
                    <div class="meta">
                        <span>{ push.time.clone() }</span>
                        {push.started_app.then(|| view! { <span class="badge">"started the app"</span> })}
                        {push.delay_ms.map(|delay| view! { <span>{ format!("+{delay} ms") }</span> })}
                    </div>
                </div>
            }
            .into_any()
        }
        support::Line::Text(text) => {
            view! {
                <p class="line">{ text.clone() }</p>
            }
            .into_any()
        }
    }
}

#[component]
fn Home() -> impl IntoView {
    let lines = RwSignal::new(Vec::<support::Line>::new());
    let permission = RwSignal::new(None::<support::PermissionUi>);
    let registration = RwSignal::new(None::<pushups::Token>);
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
                lines.update(|lines| {
                    lines.push(support::Line::Text(format!("register failed: {error}")))
                });
            }
            while let Some(event) = events.next().await {
                log(&support::describe(&event));
                if let pushups::Event::Token(token) = &event {
                    registration.set(Some(token.clone()));
                }
                lines.update(|lines| {
                    lines.push(support::line(&event, support::now_ms()));
                });
            }
        });
    });
    // The user can change the permission in Settings, so the page re-reads it when it comes
    // back to the foreground.
    let on_foreground = permission;
    let handle = window_event_listener_untyped("visibilitychange", move |_| {
        if leptos::web_sys::window()
            .and_then(|window| window.document())
            .is_some_and(|document| {
                document.visibility_state() == leptos::web_sys::VisibilityState::Visible
            })
        {
            leptos::task::spawn_local(async move {
                let status = pushups::permission_status().await;
                let answer = support::permission(status.clone());
                log(&format!("permission {answer}"));
                on_foreground.set(Some(support::permission_ui(status)));
            });
        }
    });
    on_cleanup(move || handle.remove());
    // The token's copy button copies the full token line, the page shows its short form.
    let copy = move |_event: leptos::web_sys::MouseEvent| {
        #[cfg(target_arch = "wasm32")]
        {
            spawn_local_scoped_with_cancellation(async move {
                if let Some(token) = registration.get() {
                    support::page::copy(&support::token(&token));
                }
            });
        }
    };
    // The ask button exists only while asking is offered, and the state shows on its own line.
    view! {
        <div class="app">
            <div class="header">
                <div class="mark" inner_html=support::page::MARK />
                <h2>"pushups Leptos example"</h2>
            </div>
            <div class="status">
                <div class="row">
                    <span class="label">"Transport"</span>
                    <span class="muted">
                        {move || registration.get().as_ref().map_or("—", support::transport)}
                    </span>
                </div>
                <div class="row">
                    <span class="muted">
                        {move || permission.get().map(|ui| ui.label.clone()).unwrap_or_default()}
                    </span>
                    {move || {
                        let asks = permission.get().is_some_and(|ui| ui.asks);
                        asks.then(|| {
                            view! {
                                <button on:click=move |_| {
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
                </div>
                {move || registration.get().map(|token| view! {
                    <div class="row">
                        <span class="token">{ support::shortened(&token) }</span>
                        <button class="copy" on:click={copy}>"Copy"</button>
                    </div>
                })}
            </div>
            {move || lines.get().iter().rev().map(entry_view).collect::<Vec<_>>()}
        </div>
    }
}

fn log(line: &str) {
    support::log("pushups-example", line);
}
