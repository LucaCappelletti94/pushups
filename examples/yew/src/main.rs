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
struct Lines(Vec<String>);

impl Reducible for Lines {
    type Action = String;

    fn reduce(self: Rc<Self>, line: String) -> Rc<Self> {
        let mut lines = self.0.clone();
        lines.push(line);
        Rc::new(Self(lines))
    }
}

#[function_component]
fn App() -> Html {
    let lines = use_reducer(Lines::default);
    let permission = use_state(|| None::<support::PermissionUi>);
    {
        let add = lines.dispatcher();
        let permission = permission.clone();
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
                    add.dispatch(format!("register failed: {error}"));
                }
                while let Some(event) = events.next().await {
                    log(&support::describe(&event));
                    add.dispatch(support::shown(&event));
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
    // The ask button exists only while asking is offered, and the state shows on its own line.
    let asks = permission.as_ref().is_some_and(|ui| ui.asks);
    let label = permission
        .as_ref()
        .map(|ui| ui.label.clone())
        .unwrap_or_default();
    html! {
        <div style="font-family: system-ui, sans-serif; color: #f5f7fa; padding: 20px 16px;">
            <h2>{ "pushups Yew example" }</h2>
            if asks {
                <button onclick={allow}>{ "Allow notifications" }</button>
            }
            <p>{ label }</p>
            { for lines.0.iter().map(|line| html! { <p style="font-family: monospace; font-size: 12px;">{ line }</p> }) }
        </div>
    }
}

fn log(line: &str) {
    support::log("pushups-example", line);
}
