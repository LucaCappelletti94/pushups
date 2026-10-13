//! A Tauri 2 app that receives pushes through `pushups`, with `tauri-plugin-pushups` shipping the
//! Android module and the iOS entitlement. Every event and handler call is logged with its
//! wall-clock time, to logcat under `pushups-example` on Android, so `ci/android/example.sh` can
//! time send to delivery.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use std::sync::{Mutex, PoisonError};

use pushups::{Config, Context, Event, LinuxConfig, Message, UnifiedPushConfig, WnsConfig};
use tauri::Webview;
use tauri::webview::PageLoadEvent;

#[cfg(target_os = "android")]
pushups::firebase_config!("google-services.json");

/// Installs `pushups`, sets its handler and registers, then runs the Tauri app.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    match pushups::install(config()) {
        Ok(()) => log(&format!("installed at_ms={}", now_ms())),
        Err(error) => log(&format!("install failed: {error}")),
    }
    pushups::set_handler(|event| {
        log(&describe(&event));
        page_show(shown(&event));
    });
    if let Err(error) = pushups::register() {
        log(&format!("register failed: {error}"));
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_pushups::init())
        .on_page_load(|webview, payload| {
            if payload.event() == PageLoadEvent::Finished {
                page_loaded(webview.clone());
            }
        })
        .run(tauri::generate_context!())
        .expect("the Tauri app runs");
}

/// The lines the page shows, and the page once it has loaded.
static PAGE: Mutex<Page> = Mutex::new(Page {
    lines: Vec::new(),
    webview: None,
});

struct Page {
    lines: Vec<String>,
    webview: Option<Webview>,
}

/// Adds the line to the page, now if it has loaded, else when it does.
fn page_show(line: String) {
    let mut page = PAGE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(webview) = &page.webview {
        show_in(webview, &line);
    }
    page.lines.push(line);
}

/// Shows every line so far in the page that just loaded, and the later ones as they come.
fn page_loaded(webview: Webview) {
    let mut page = PAGE.lock().unwrap_or_else(PoisonError::into_inner);
    for line in &page.lines {
        show_in(&webview, line);
    }
    page.webview = Some(webview);
}

fn show_in(webview: &Webview, line: &str) {
    let text = serde_json::to_string(line).expect("a string serialises");
    if let Err(error) = webview.eval(format!("pushupsShow({text})")) {
        log(&format!("page update failed: {error}"));
    }
}

/// The event as the page shows it, a push as the `body` its payload carries.
fn shown(event: &Event) -> String {
    match event {
        Event::Token(_) => "Registered for pushes".to_owned(),
        Event::RegistrationFailed(error) => format!("Registration failed: {error}"),
        Event::Message(message) => {
            let data: serde_json::Value =
                serde_json::from_slice(&message.payload).unwrap_or_default();
            match data.get("body").and_then(serde_json::Value::as_str) {
                Some(body) => format!("Push: {body}"),
                None => format!("Push: {}", String::from_utf8_lossy(&message.payload)),
            }
        }
        Event::MessagesDropped => "Some pushes were dropped".to_owned(),
    }
}

/// The app's configuration from build-time keys, `PUSHUPS_VAPID_PUBLIC_KEY` (base64url) for UnifiedPush on Linux and Android and `PUSHUPS_WNS_REMOTE_ID` for Windows.
fn config() -> Config {
    // The bundle identifier, which also names the app on the Linux session bus.
    let mut config = Config::new().linux(LinuxConfig::new("rs.pushups.example"));
    if let Some(remote_id) = option_env!("PUSHUPS_WNS_REMOTE_ID") {
        config = config.windows(WnsConfig::new(remote_id));
    }
    let key = option_env!("PUSHUPS_VAPID_PUBLIC_KEY")
        .and_then(|key| URL_SAFE_NO_PAD.decode(key).ok())
        .and_then(|key| <[u8; 65]>::try_from(key).ok());
    if let Some(key) = key {
        config = config.unified_push(UnifiedPushConfig::new(key));
    }
    config
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
            "ui message at_ms={at} started_app={} {} payload={}",
            message.started_app,
            timing(message, at),
            String::from_utf8_lossy(&message.payload)
        ),
        Event::MessagesDropped => format!("ui messages dropped at_ms={at}"),
    }
}

/// Runs off the main thread for every push the system delivers.
#[pushups::background_handler]
fn on_push(_context: Context, message: Message) {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    popup(&message);
    let entered = now_ms();
    log(&format!(
        "handler started_app={} entered_at_ms={entered} {} payload={}",
        message.started_app,
        timing(&message, entered),
        String::from_utf8_lossy(&message.payload)
    ));
}

/// Shows the push as a desktop popup, which Windows and Linux leave to the app.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn popup(message: &Message) {
    let data: serde_json::Value = serde_json::from_slice(&message.payload).unwrap_or_default();
    let text = |name: &str| data.get(name).and_then(serde_json::Value::as_str);
    let shown = notify_rust::Notification::new()
        .summary(text("title").unwrap_or("pushups"))
        .body(text("body").unwrap_or("A push arrived"))
        .show();
    if let Err(error) = shown {
        log(&format!("popup failed: {error}"));
    }
}

/// The push's `seq` and its latency from the sender's `sent_at_ms` to `at`, for a JSON payload
/// such as an FCM data map.
fn timing(message: &Message, at: u128) -> String {
    let data: serde_json::Value = serde_json::from_slice(&message.payload).unwrap_or_default();
    let field = |name: &str| data.get(name).and_then(serde_json::Value::as_str);
    let sent = field("sent_at_ms").and_then(|sent| sent.parse::<u128>().ok());
    format!(
        "seq={} sent_at_ms={} since_sent_ms={}",
        field("seq").unwrap_or("-"),
        sent.map_or_else(|| "-".to_owned(), |sent| sent.to_string()),
        sent.map_or_else(
            || "-".to_owned(),
            |sent| at.saturating_sub(sent).to_string()
        ),
    )
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis())
}

/// Prints the line, and appends it to `pushups-example.log` in the temporary directory, which an
/// iOS app's container keeps for `devicectl` to copy, since its standard output reaches nothing.
#[cfg(not(target_os = "android"))]
fn log(line: &str) {
    use std::io::Write;

    let line = format!("pushups-tauri-example {line}");
    println!("{line}");
    let path = std::env::temp_dir().join("pushups-example.log");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        // One write per line, so lines from the handler thread and the UI never interleave.
        let _ = file.write_all(format!("{line}\n").as_bytes());
    }
}

/// Writes the line to logcat under the tag `pushups-example`, which `ci/android/example.sh` reads.
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
