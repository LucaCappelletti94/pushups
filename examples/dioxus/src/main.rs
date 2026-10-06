//! Receives pushes through `pushups` and logs every step with its wall-clock time, under the
//! logcat tag `pushups-example` on Android and in the console on the web, so a device proof can
//! time send to delivery.
//!
//! On the web, `PUSHUPS_VAPID_PUBLIC_KEY` (base64url) at build time sets the VAPID key, and
//! `PUSHUPS_WEB_WORKER=static` registers the static worker in place of the Rust handler.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use dioxus::prelude::*;
use futures_util::StreamExt;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use pushups::{AndroidContext, Config, Event, Message, Notification, Permission, WebPushConfig};

#[cfg(target_os = "android")]
#[manganis::ffi("../../android")]
extern "Kotlin" {
    pub type Pushups;
}

pushups::firebase_config!("google-services.json");

/// Where the background handler sends its request, forwarded to the host by
/// `adb reverse tcp:8787 tcp:8787`.
const CATCH_UP: &str = "127.0.0.1:8787";

fn main() {
    if pushups::in_service_worker() {
        if let Err(error) = pushups::serve_service_worker(notification_for) {
            log(&format!("serving the service worker failed: {error}"));
        }
        return;
    }
    if let Err(error) = pushups::install(config()) {
        log(&format!("install failed: {error}"));
    }
    dioxus::launch(App);
}

fn config() -> Config {
    let key = option_env!("PUSHUPS_VAPID_PUBLIC_KEY")
        .and_then(|key| URL_SAFE_NO_PAD.decode(key).ok())
        .and_then(|key| <[u8; 65]>::try_from(key).ok());
    let Some(key) = key else {
        return Config::new();
    };
    let web = WebPushConfig::new(key);
    Config::new().web(match option_env!("PUSHUPS_WEB_WORKER") {
        Some("static") => web,
        _ => web.rust_handler(),
    })
}

/// Runs in the service worker for every push, and decides what the user sees.
async fn notification_for(message: Message) -> Notification {
    let at = now_ms();
    let line = timing(&message, at);
    log(&format!("worker push at_ms={at} {line}"));
    Notification::new("pushups from Rust")
        .body(format!("built in the service worker, {line}"))
        .navigate("/")
}

#[component]
fn App() -> Element {
    let mut lines = use_signal(Vec::<String>::new);
    let mut permission = use_signal(|| None::<String>);
    use_future(move || async move {
        let mut events = pushups::events();
        if let Err(error) = pushups::register() {
            lines.write().push(format!("register failed: {error}"));
        }
        while let Some(event) = events.next().await {
            let line = describe(&event);
            log(&line);
            lines.write().push(line);
        }
    });
    rsx! {
        div { style: "font-family: sans-serif; padding: 16px;",
            h3 { "pushups example" }
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
            p { "permission: " {permission.read().clone().unwrap_or_default()} }
            for line in lines.read().iter() {
                p { "{line}" }
            }
        }
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

/// Runs in the process the push woke, before any UI exists.
#[pushups::background_handler]
fn on_push(_context: AndroidContext, message: Message) {
    let entered = now_ms();
    let started = Instant::now();
    let fetched = catch_up();
    let fetch_ms = started.elapsed().as_millis();
    log(&format!(
        "handler started_app={} entered_at_ms={entered} {} fetch_ms={fetch_ms} fetch={fetched}",
        message.started_app,
        timing(&message, entered),
    ));
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
        sent.map_or_else(|| "-".to_owned(), |sent| at.saturating_sub(sent).to_string()),
    )
}

/// One HTTP request to the host, standing in for an app fetching what the push announced.
fn catch_up() -> String {
    let attempt = || -> std::io::Result<String> {
        let mut stream = TcpStream::connect(CATCH_UP)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.write_all(b"GET /catch-up HTTP/1.0\r\nHost: pushups\r\n\r\n")?;
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        Ok(response.lines().next().unwrap_or("empty response").to_owned())
    };
    attempt().unwrap_or_else(|error| format!("failed: {error}"))
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

#[cfg(target_os = "android")]
fn log(line: &str) {
    use std::ffi::{CString, c_char, c_int};

    #[link(name = "log")]
    unsafe extern "C" {
        fn __android_log_write(priority: c_int, tag: *const c_char, text: *const c_char) -> c_int;
    }
    const INFO: c_int = 4;
    let Ok(text) = CString::new(line.replace('\0', " ")) else {
        return;
    };
    // SAFETY: both strings are NUL-terminated and stay alive for the call, and liblog only
    // reads them.
    unsafe {
        __android_log_write(INFO, c"pushups-example".as_ptr(), text.as_ptr());
    }
}

#[cfg(target_arch = "wasm32")]
fn log(line: &str) {
    web_sys::console::log_1(&format!("pushups-example {line}").into());
}

#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
fn log(line: &str) {
    println!("pushups-example {line}");
}
