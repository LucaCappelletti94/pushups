//! The app `ci/web/harness.mjs` drives through `pushups` in a real browser. The page calls the exports below, the same wasm serves the Rust handler in the service worker, and both expose their coverage counters as `globalThis.pushupsProbeCoverage`.
#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use js_sys::{Array, Object, Reflect, Uint8Array};
use parking_lot::Mutex;
use pushups::{Config, Event, Message, Notification, Permission, Token, WebPushConfig};
use wasm_bindgen::prelude::*;

/// What the handler received, in arrival order, until the harness takes it.
static EVENTS: Mutex<Vec<Event>> = Mutex::new(Vec::new());

#[wasm_bindgen(start)]
fn start() -> Result<(), JsValue> {
    let capture = Closure::<dyn Fn() -> Result<Uint8Array, JsValue>>::new(|| {
        let mut profile = Vec::new();
        // SAFETY: the page and the worker are single-threaded, so no instrumented code runs while the counters are read.
        unsafe { minicov::capture_coverage(&mut profile) }
            .map_err(|_| JsValue::from_str("capturing the coverage counters failed"))?;
        Ok(Uint8Array::from(profile.as_slice()))
    });
    Reflect::set(
        &js_sys::global(),
        &JsValue::from_str("pushupsProbeCoverage"),
        capture.as_ref(),
    )?;
    // The harness reads the counters until the page or the worker ends.
    capture.forget();
    if pushups::in_service_worker() {
        pushups::serve_service_worker(notification_for).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Builds every push's notification in the service worker, with every field set.
async fn notification_for(message: Message) -> Notification {
    let text = String::from_utf8_lossy(&message.payload).into_owned();
    Notification::new("probe from Rust")
        .body(text)
        .navigate("/?from=rust")
        .tag("probe")
        .icon("/icon.png")
}

/// The errors of calls made out of order or in the wrong context, before `install`.
#[wasm_bindgen]
#[must_use]
pub fn misuse() -> Array {
    let outcomes = [
        format!("in_service_worker={}", pushups::in_service_worker()),
        format!("register={:?}", pushups::register()),
        format!("install={:?}", pushups::install(Config::new())),
        format!(
            "serve_service_worker={:?}",
            pushups::serve_service_worker(notification_for)
        ),
    ];
    outcomes
        .iter()
        .map(|outcome| JsValue::from_str(outcome))
        .collect()
}

/// Installs the backend with the static worker at `worker_path`, or the Rust handler.
///
/// # Errors
///
/// The error of `pushups::install`.
#[wasm_bindgen]
pub fn install(
    vapid_public_key: &[u8],
    rust_handler: bool,
    worker_path: Option<String>,
) -> Result<(), JsError> {
    let key = <[u8; 65]>::try_from(vapid_public_key)
        .map_err(|_| JsError::new("a VAPID public key is 65 bytes"))?;
    let mut web = WebPushConfig::new(key);
    if let Some(path) = worker_path {
        web = web.service_worker_path(path);
    }
    if rust_handler {
        web = web.rust_handler();
    }
    pushups::install(Config::new().web(web)).map_err(|error| JsError::new(&error.to_string()))
}

/// Sets the handler, which records every event for `take_events`.
#[wasm_bindgen]
pub fn listen() {
    pushups::set_handler(|event| EVENTS.lock().push(event));
}

/// Subscribes, reporting the token or the failure as an event.
///
/// # Errors
///
/// The error of `pushups::register`.
#[wasm_bindgen]
pub fn register() -> Result<(), JsError> {
    pushups::register().map_err(|error| JsError::new(&error.to_string()))
}

/// Asks for the notification permission and names the answer.
///
/// # Errors
///
/// The error of `pushups::request_permission`.
#[wasm_bindgen(js_name = requestPermission)]
pub async fn request_permission() -> Result<String, JsError> {
    let answer = pushups::request_permission()
        .await
        .map_err(|error| JsError::new(&error.to_string()))?;
    Ok(match answer {
        Permission::Granted => "granted".to_owned(),
        Permission::Denied => "denied".to_owned(),
        Permission::Dismissed => "dismissed".to_owned(),
        other => format!("{other:?}"),
    })
}

/// Hands over and forgets every event received so far, as plain objects.
///
/// # Errors
///
/// A property that cannot be set, which a plain object never refuses.
#[wasm_bindgen(js_name = takeEvents)]
pub fn take_events() -> Result<Array, JsValue> {
    std::mem::take(&mut *EVENTS.lock())
        .iter()
        .map(event_object)
        .collect()
}

fn event_object(event: &Event) -> Result<JsValue, JsValue> {
    let object = Object::new();
    let set = |name: &str, value: JsValue| Reflect::set(&object, &JsValue::from_str(name), &value);
    match event {
        Event::Token(token @ Token::WebPush { endpoint, .. }) => {
            set("kind", "token".into())?;
            set("endpoint", endpoint.into())?;
            if let Some(keys) = token.web_push_keys() {
                set("p256dh", keys.p256dh.into())?;
                set("auth", keys.auth.into())?;
            }
        }
        Event::Token(token) => {
            set("kind", "token".into())?;
            set("other", format!("{token:?}").into())?;
        }
        Event::RegistrationFailed(error) => {
            set("kind", "registrationFailed".into())?;
            set("error", error.to_string().into())?;
        }
        Event::Message(message) => {
            set("kind", "message".into())?;
            set(
                "payload",
                String::from_utf8_lossy(&message.payload)
                    .into_owned()
                    .into(),
            )?;
            set("startedApp", message.started_app.into())?;
        }
        Event::MessagesDropped => {
            set("kind", "messagesDropped".into())?;
        }
    }
    Ok(object.into())
}
