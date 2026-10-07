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
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use pushups::{Config, Context, Event, Message, Notification, Permission, WebPushConfig};

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
    match pushups::install(config()) {
        Ok(()) => log(&format!("installed at_ms={}", now_ms())),
        Err(error) => log(&format!("install failed: {error}")),
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
    let favicon = use_hook(|| format!("data:image/svg+xml;base64,{}", STANDARD.encode(FAVICON)));
    use_future(move || async move {
        log(&format!("ui mounted at_ms={}", now_ms()));
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
        document::Style { {PAGE} }
        document::Link { rel: "icon", href: favicon }
        div { class: "app",
            div { class: "header",
                div { class: "mark", dangerous_inner_html: MARK }
                h2 { "pushups example" }
            }
            div { class: "actions",
                button {
                    onclick: move |_| async move {
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
                        permission.set(Some(answer));
                    },
                    "Allow notifications"
                }
                if cfg!(target_arch = "wasm32") {
                    button { onclick: move |_| reload_page(), "Reload" }
                    button {
                        onclick: move |_| copy_text(&lines.read().join("\n")),
                        "Copy log"
                    }
                }
            }
            p { class: "muted", "permission: " {permission.read().clone().unwrap_or_default()} }
            for line in lines.read().iter() {
                p { class: "line", "{line}" }
            }
        }
    }
}

/// The page's styles in the brand's colours from `assets/brand`, a navy ground and the mark's red-to-orange gradient.
const PAGE: &str = "
:root { --navy: #0e1824; --surface: #172436; --text: #f5f7fa; --muted: #9aa6b8; --gradient: linear-gradient(135deg, #fa1e13, #f67017); }
html, body { margin: 0; min-height: 100%; background: var(--navy); }
.app { font-family: -apple-system, system-ui, sans-serif; color: var(--text); padding: 20px 16px; max-width: 640px; margin: 0 auto; box-sizing: border-box; }
.header { display: flex; align-items: center; gap: 12px; margin-bottom: 16px; }
.header h2 { margin: 0; font-size: 26px; font-weight: 700; }
.mark { width: 56px; flex: none; }
.mark svg { display: block; width: 100%; height: auto; }
.actions { display: flex; gap: 8px; flex-wrap: wrap; }
.actions button { font-size: 16px; font-weight: 600; color: #ffffff; padding: 10px 16px; border: none; border-radius: 10px; background: var(--gradient); }
.muted { color: var(--muted); }
.line { font-family: ui-monospace, monospace; font-size: 12px; overflow-wrap: anywhere; background: var(--surface); padding: 8px 10px; border-radius: 8px; margin: 8px 0; }
";

/// The brand mark, drawn inline so every platform shows it without an asset pipeline.
const MARK: &str = include_str!("../../../assets/brand/mark.svg");

/// The browser tab's icon, linked as a `data:` URL since `public_dir` serves only the crate's `js/`.
const FAVICON: &[u8] = include_bytes!("../../../assets/brand/favicon.svg");

/// Reloads the page, the only way to restart a home-screen web app on iOS from inside it.
#[cfg(target_arch = "wasm32")]
fn reload_page() {
    let _ = js_sys::Function::new_no_args("location.reload()").call0(&js_sys::global());
}

#[cfg(not(target_arch = "wasm32"))]
fn reload_page() {}

/// Puts `text` on the clipboard, so a phone can paste the log into a message.
#[cfg(target_arch = "wasm32")]
fn copy_text(text: &str) {
    let copy = js_sys::Function::new_with_args("text", "navigator.clipboard.writeText(text)");
    let _ = copy.call1(&js_sys::global(), &js_sys::JsString::from(text));
}

#[cfg(not(target_arch = "wasm32"))]
fn copy_text(_text: &str) {}

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
fn on_push(_context: Context, message: Message) {
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

/// Prints the line, and appends it to `pushups-example.log` in the temporary directory, which outlives a process the system launched for a push or a tap with nothing reading its output.
#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
fn log(line: &str) {
    let line = format!("pushups-example {line}");
    println!("{line}");
    let path = std::env::temp_dir().join("pushups-example.log");
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        // One write per line, so lines from the handler thread and the UI never interleave.
        let _ = file.write_all(format!("{line}\n").as_bytes());
    }
}
