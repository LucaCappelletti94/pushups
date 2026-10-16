//! A Yew app built with Trunk that receives pushes through `pushups`, logging every step to the
//! console with its wall-clock time, so the CI check can time send to delivery.
//!
//! `PUSHUPS_VAPID_PUBLIC_KEY` (base64url) at build time sets the app server's VAPID key.
//! `PUSHUPS_WEB_WORKER=static` registers the static worker, which `build.rs` writes from
//! `pushups::SERVICE_WORKER`, in place of the Rust handler, which runs from the `sw.js` entry.

use std::rc::Rc;

use futures_util::StreamExt;
use pushups::{Config, Event, Message, Notification, Permission, VapidPublicKey, WebPushConfig};
use pushups_example_support as support;
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
    let Some(key) = option_env!("PUSHUPS_VAPID_PUBLIC_KEY") else {
        return Config::new();
    };
    let key = VapidPublicKey::from_base64url(key).expect("PUSHUPS_VAPID_PUBLIC_KEY is a VAPID key");
    let web = WebPushConfig::new(key);
    Config::new().web(match option_env!("PUSHUPS_WEB_WORKER") {
        Some("static") => web,
        _ => web.rust_handler().service_worker_path("/sw.js"),
    })
}

/// Runs in the service worker for every push, and decides what the user sees.
async fn notification_for(message: Message) -> Notification {
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
    let permission = use_state(String::new);
    {
        let add = lines.dispatcher();
        use_effect_with((), move |()| {
            wasm_bindgen_futures::spawn_local(async move {
                log(&format!("ui mounted at_ms={}", support::now_ms()));
                let mut events = pushups::events();
                if let Err(error) = pushups::register() {
                    add.dispatch(format!("register failed: {error}"));
                }
                while let Some(event) = events.next().await {
                    log(&describe(&event));
                    add.dispatch(support::shown(&event));
                }
            });
        });
    }
    let allow = {
        let permission = permission.clone();
        Callback::from(move |_| {
            let permission = permission.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let answer = match pushups::request_permission().await {
                    Ok(Permission::Granted) => "granted".to_owned(),
                    Ok(Permission::Denied) => "denied".to_owned(),
                    Ok(other) => format!("{other:?}"),
                    Err(error) => format!("failed: {error}"),
                };
                log(&format!("permission {answer}"));
                permission.set(answer);
            });
        })
    };
    html! {
        <div style="font-family: system-ui, sans-serif; color: #f5f7fa; padding: 20px 16px;">
            <h2>{ "pushups Yew example" }</h2>
            <button onclick={allow}>{ "Allow notifications" }</button>
            <p>{ format!("permission: {}", *permission) }</p>
            { for lines.0.iter().map(|line| html! { <p style="font-family: monospace; font-size: 12px;">{ line }</p> }) }
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
