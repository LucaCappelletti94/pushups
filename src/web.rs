//! The Web Push backend, with `js/pushups-sw.js` as its service worker.

use std::cell::RefCell;

use js_sys::{Array, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, future_to_promise, spawn_local};
use web_sys::ServiceWorkerRegistration;

use crate::dispatch::DISPATCHER;
use crate::web_parts::{WorkerMode, permission_from, web_push_token, worker_script_url};
use crate::{Config, Error, Event, Message, Notification, Permission, WebPushConfig};

// wasm-bindgen reads the snippet without telling cargo, so a change to it would not rebuild.
const _: &str = include_str!("../js/pushups-sw.js");

#[wasm_bindgen(module = "/js/pushups-sw.js")]
extern "C" {
    #[wasm_bindgen(js_name = inServiceWorker)]
    fn shim_in_service_worker() -> bool;
    #[wasm_bindgen(js_name = serve)]
    fn shim_serve(handler: &js_sys::Function);
    #[wasm_bindgen(js_name = activeRegistration, catch)]
    fn shim_active_registration(registration: &JsValue) -> Result<Promise, JsValue>;
    #[wasm_bindgen(js_name = subscribe, catch)]
    fn shim_subscribe(
        registration: &ServiceWorkerRegistration,
        key: &Uint8Array,
    ) -> Result<Promise, JsValue>;
    #[wasm_bindgen(js_name = drain, catch)]
    fn shim_drain() -> Result<Promise, JsValue>;
    #[wasm_bindgen(js_name = onDrainRequest)]
    fn shim_on_drain_request(callback: &js_sys::Function);
}

#[wasm_bindgen]
extern "C" {
    /// The URL of the module this import lives in, the app's wasm-bindgen glue.
    #[wasm_bindgen(thread_local_v2, js_namespace = ["import", "meta"], js_name = url)]
    static GLUE_URL: String;
}

/// What `install` set up on this page.
struct Page {
    config: WebPushConfig,
    registration: Promise,
    handler_set: bool,
}

thread_local! {
    static PAGE: RefCell<Option<Page>> = const { RefCell::new(None) };
}

fn js_error(value: &JsValue) -> Error {
    Error::Platform(value.dyn_ref::<js_sys::Error>().map_or_else(
        || format!("{value:?}"),
        |error| String::from(error.message()),
    ))
}

pub(crate) fn in_service_worker() -> bool {
    shim_in_service_worker()
}

pub(crate) fn install(config: Config) -> Result<(), Error> {
    let web = config.web.ok_or(Error::NotConfigured)?;
    let window = web_sys::window().ok_or(Error::Unsupported)?;
    let navigator = window.navigator();
    let has_push = Reflect::has(&window, &JsValue::from_str("PushManager")).unwrap_or(false);
    if !has_push || !Reflect::has(&navigator, &JsValue::from_str("serviceWorker")).unwrap_or(false)
    {
        return Err(Error::Unsupported);
    }
    let script = if web.rust_handler {
        worker_script_url(&GLUE_URL.with(Clone::clone), WorkerMode::Rust)
    } else {
        worker_script_url(&web.service_worker_path, WorkerMode::Static)
    };
    let options = web_sys::RegistrationOptions::new();
    options.set_type("module");
    let registration = navigator
        .service_worker()
        .register_with_options(&script, &options);
    let drain_request = Closure::<dyn Fn()>::new(|| {
        if PAGE.with_borrow(|page| page.as_ref().is_some_and(|page| page.handler_set)) {
            spawn_local(drain());
        }
    });
    shim_on_drain_request(drain_request.as_ref().unchecked_ref());
    // The listener lives as long as the page.
    drain_request.forget();
    PAGE.set(Some(Page {
        config: web,
        registration,
        handler_set: false,
    }));
    Ok(())
}

pub(crate) fn register() -> Result<(), Error> {
    let (registration, key) = PAGE.with_borrow(|page| {
        page.as_ref()
            .map(|page| {
                (
                    page.registration.clone(),
                    Uint8Array::from(page.config.vapid_public_key.as_slice()),
                )
            })
            .ok_or(Error::NotConfigured)
    })?;
    spawn_local(async move {
        let event = match subscribe(registration, key).await {
            Ok(token) => Event::Token(token),
            Err(error) => Event::RegistrationFailed(error),
        };
        DISPATCHER.emit(event);
    });
    Ok(())
}

async fn subscribe(registration: Promise, key: Uint8Array) -> Result<crate::Token, Error> {
    let registration = JsFuture::from(registration)
        .await
        .map_err(|e| js_error(&e))?;
    let active = shim_active_registration(&registration).map_err(|e| js_error(&e))?;
    let registration: ServiceWorkerRegistration = JsFuture::from(active)
        .await
        .map_err(|e| js_error(&e))?
        .unchecked_into();
    let subscribed = shim_subscribe(&registration, &key).map_err(|e| js_error(&e))?;
    let record = JsFuture::from(subscribed).await.map_err(|e| js_error(&e))?;
    token_from_record(&record)
}

fn field(record: &JsValue, name: &str) -> Result<JsValue, Error> {
    Reflect::get(record, &JsValue::from_str(name)).map_err(|e| js_error(&e))
}

fn token_from_record(record: &JsValue) -> Result<crate::Token, Error> {
    let endpoint = field(record, "endpoint")?
        .as_string()
        .ok_or_else(|| Error::Platform("subscription without an endpoint".to_owned()))?;
    let p256dh = Uint8Array::new(&field(record, "p256dh")?).to_vec();
    let auth = Uint8Array::new(&field(record, "auth")?).to_vec();
    let expires = field(record, "expires")?.as_f64();
    web_push_token(endpoint, &p256dh, &auth, expires)
}

fn event_from_record(record: &JsValue) -> Result<Event, Error> {
    match field(record, "kind")?.as_string().as_deref() {
        Some("message") => Ok(Event::Message(Message {
            payload: Uint8Array::new(&field(record, "payload")?).to_vec(),
            started_app: field(record, "startedApp")?.is_truthy(),
        })),
        Some("token") => token_from_record(record).map(Event::Token),
        other => Err(Error::Platform(format!("unknown queued record {other:?}"))),
    }
}

/// Hands every queued push and token to the handler, in arrival order.
async fn drain() {
    let records = match shim_drain() {
        Ok(promise) => JsFuture::from(promise).await,
        Err(error) => Err(error),
    };
    // A failed read leaves the queue in place for the next drain.
    let Ok(records) = records else {
        web_sys::console::error_1(&JsValue::from_str("pushups: reading the push queue failed"));
        return;
    };
    for record in Array::from(&records).iter() {
        match event_from_record(&record) {
            Ok(event) => DISPATCHER.emit(event),
            Err(error) => web_sys::console::error_1(&JsValue::from_str(&format!(
                "pushups: dropped an unreadable queued record: {error}"
            ))),
        }
    }
}

pub(crate) fn handler_set() {
    let bound =
        PAGE.with_borrow_mut(|page| page.as_mut().map(|page| page.handler_set = true).is_some());
    if bound {
        spawn_local(drain());
    }
}

pub(crate) async fn request_permission() -> Result<Permission, Error> {
    let window = web_sys::window().ok_or(Error::Unsupported)?;
    if !Reflect::has(&window, &JsValue::from_str("Notification")).unwrap_or(false) {
        return Err(Error::Unsupported);
    }
    let asked = web_sys::Notification::request_permission().map_err(|e| js_error(&e))?;
    let answer = JsFuture::from(asked).await.map_err(|e| js_error(&e))?;
    permission_from(&answer.as_string().unwrap_or_default())
}

fn notification_object(notification: &Notification) -> Object {
    let object = Object::new();
    let fields = [
        ("title", Some(notification.title.as_str())),
        ("body", notification.body.as_deref()),
        ("navigate", notification.navigate.as_deref()),
        ("tag", notification.tag.as_deref()),
        ("icon", notification.icon.as_deref()),
    ];
    for (name, value) in fields {
        if let Some(value) = value {
            // Setting a string on a fresh plain object cannot throw.
            let _ = Reflect::set(&object, &JsValue::from_str(name), &JsValue::from_str(value));
        }
    }
    object
}

pub(crate) fn serve_service_worker<H, F>(handler: H) -> Result<(), Error>
where
    H: Fn(Message) -> F + 'static,
    F: Future<Output = Notification> + 'static,
{
    if !shim_in_service_worker() {
        return Err(Error::NotInServiceWorker);
    }
    let handler = Closure::<dyn Fn(Uint8Array) -> Promise>::new(move |payload: Uint8Array| {
        let shown = handler(Message {
            payload: payload.to_vec(),
            started_app: false,
        });
        future_to_promise(async move { Ok(notification_object(&shown.await).into()) })
    });
    shim_serve(handler.as_ref().unchecked_ref());
    // The worker keeps the handler for its whole life.
    handler.forget();
    Ok(())
}
