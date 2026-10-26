//! A Yew app built with Trunk that receives pushes through `pushups`, logging every step to the
//! console with its wall-clock time, so the CI check can time send to delivery.
//!
//! `PUSHUPS_VAPID_PUBLIC_KEY` (base64url) at build time sets the app server's VAPID key.
//! `PUSHUPS_WEB_WORKER=static` registers the static worker, which `build.rs` writes from
//! `pushups::SERVICE_WORKER`, in place of the Rust handler, which runs from the `sw.js` entry.

use std::rc::Rc;

use futures_util::StreamExt;
use pushups::{Config, Message, Notification};
use pushups_example_support as support;
use wasm_bindgen::JsCast;
use yew::prelude::*;

fn main() {
    if pushups::in_service_worker() {
        if let Err(error) = pushups::serve_service_worker(notification_for) {
            log(&format!("serving the service worker failed: {error}"));
        }
        return;
    }
    match pushups::install(config()) {
        Ok(()) => log(&format!("installed at_ms={}", support::now_ms())),
        Err(error) => log(&format!("install failed: {error}")),
    }
    yew::Renderer::<App>::new().render();
}

fn config() -> Config {
    support::web_push(Some("/sw.js")).map_or_else(Config::new, |web| Config::new().web(web))
}

/// Runs in the service worker for every push, and decides what the user sees.
async fn notification_for(message: Message) -> Notification {
    support::worker_notification(&message, log)
}

/// The page's log, newest last.
#[derive(Default, PartialEq)]
struct Lines(Vec<support::Line>);

impl Reducible for Lines {
    type Action = support::Line;

    fn reduce(self: Rc<Self>, entry: support::Line) -> Rc<Self> {
        let mut lines = self.0.clone();
        lines.push(entry);
        Rc::new(Self(lines))
    }
}

/// The line of the list the page shows, a push card or a compact event.
fn entry_view(entry: &support::Line) -> Html {
    match entry {
        support::Line::Push(push) => html! {
            <div class="card">
                <p class="title">{ push.title.clone() }</p>
                <p class="body">{ push.body.clone() }</p>
                <div class="meta">
                    <span>{ push.time.clone() }</span>
                    if push.started_app {
                        <span class="badge">{ "started the app" }</span>
                    }
                    if let Some(delay) = push.delay_ms {
                        <span>{ format!("+{delay} ms") }</span>
                    }
                </div>
            </div>
        },
        support::Line::Text(text) => html! {
            <p class="line">{ text.clone() }</p>
        },
    }
}

#[function_component]
fn App() -> Html {
    let lines = use_reducer(Lines::default);
    let permission = use_state(|| None::<support::PermissionUi>);
    let registration = use_state(|| None::<pushups::Token>);
    {
        let add = lines.dispatcher();
        let permission = permission.clone();
        let registration = registration.clone();
        use_effect_with((), move |()| {
            wasm_bindgen_futures::spawn_local(async move {
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
                    add.dispatch(support::Line::Text(format!("register failed: {error}")));
                }
                while let Some(event) = events.next().await {
                    log(&support::describe(&event));
                    if let pushups::Event::Token(token) = &event {
                        registration.set(Some(token.clone()));
                    }
                    add.dispatch(support::line(&event, support::now_ms()));
                }
            });
        });
    }
    // The user can change the permission in Settings, so the page re-reads it when it comes back
    // to the foreground.
    {
        let permission = permission.clone();
        use_effect_with((), move |()| {
            let Some(document) = web_sys::window().and_then(|window| window.document()) else {
                return;
            };
            let on_foreground = permission.clone();
            // A clone for the registration, since the closure takes the document by move.
            let listener = document.clone();
            let closure = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
                if document.visibility_state() == web_sys::VisibilityState::Visible {
                    let on_foreground = on_foreground.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        let status = pushups::permission_status().await;
                        let answer = support::permission(status.clone());
                        log(&format!("permission {answer}"));
                        on_foreground.set(Some(support::permission_ui(status)));
                    });
                }
            });
            let _ = listener.add_event_listener_with_callback(
                "visibilitychange",
                closure.as_ref().unchecked_ref(),
            );
            // The listener keeps the closure alive for the life of the page, so the Rust copy
            // must not drop it.
            closure.forget();
        });
    }
    let allow = {
        let permission = permission.clone();
        Callback::from(move |_| {
            let permission = permission.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let status = pushups::request_permission().await;
                let answer = support::permission(status.clone());
                log(&format!("permission {answer}"));
                permission.set(Some(support::permission_ui(status)));
            });
        })
    };
    // The token's copy button copies the full token line, the page shows its short form.
    let copy = {
        #[cfg(target_arch = "wasm32")]
        let registration = registration.clone();
        Callback::from(move |_| {
            // The clipboard lives in the browser, so the host build keeps an empty body.
            #[cfg(target_arch = "wasm32")]
            {
                let Some(token) = &*registration else {
                    return;
                };
                support::page::copy(&support::token(token));
            }
        })
    };
    // The ask button exists only while asking is offered, and the state shows on its own line.
    let asks = permission.as_ref().is_some_and(|ui| ui.asks);
    let label = permission
        .as_ref()
        .map(|ui| ui.label.clone())
        .unwrap_or_default();
    let entries: Vec<Html> = lines.0.iter().rev().map(entry_view).collect();
    html! {
        <>
            <style>{ support::page::CSS }</style>
            <div class="app">
                <div class="header">
                    <div class="mark">
                        { Html::from_html_unchecked(AttrValue::from(support::page::MARK)) }
                    </div>
                    <h2>{ "pushups Yew example" }</h2>
                </div>
                <div class="status">
                    <div class="row">
                        <span class="label">{ "Transport" }</span>
                        <span class="muted">{ registration.as_ref().map_or("—", support::transport) }</span>
                    </div>
                    <div class="row">
                        <span class="muted">{ label }</span>
                        if asks {
                            <button onclick={allow}>{ "Allow notifications" }</button>
                        }
                    </div>
                    if registration.is_some() {
                        <div class="row">
                            <span class="token">{ registration.as_ref().map_or_default(support::shortened) }</span>
                            <button class="copy" onclick={copy}>{ "Copy" }</button>
                        </div>
                    }
                </div>
                { entries }
            </div>
        </>
    }
}

fn log(line: &str) {
    support::log("pushups-example", line);
}
