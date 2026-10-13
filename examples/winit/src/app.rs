//! The app both entry points run, `main` on desktop, iOS and the web and `android_main` on Android.

use std::sync::mpsc::{Receiver, channel};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use pushups::{Config, Context, Event, Message};

// The Linux and Windows sub-configs, unused on the web, which takes its key for `WebPushConfig` instead.
#[cfg(not(target_arch = "wasm32"))]
use pushups::{LinuxConfig, UnifiedPushConfig, WnsConfig};
// `std`'s clock panics on `wasm32-unknown-unknown`, where `web-time` reads `Date.now()` instead.
#[cfg(target_arch = "wasm32")]
use web_time::{SystemTime, UNIX_EPOCH};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

/// Installs `pushups`, before the event loop exists.
pub(crate) fn install() {
    match pushups::install(config()) {
        Ok(()) => log(&format!("installed at_ms={}", now_ms())),
        Err(error) => log(&format!("install failed: {error}")),
    }
}

#[cfg(not(target_arch = "wasm32"))]
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

/// The web configuration from `PUSHUPS_VAPID_PUBLIC_KEY` (base64url). `PUSHUPS_WEB_WORKER=static` registers the static worker, which `build.rs` writes from `pushups::SERVICE_WORKER`, in place of the Rust handler, which runs from the `sw.js` entry.
#[cfg(target_arch = "wasm32")]
fn config() -> Config {
    let key = option_env!("PUSHUPS_VAPID_PUBLIC_KEY")
        .and_then(|key| URL_SAFE_NO_PAD.decode(key).ok())
        .and_then(|key| <[u8; 65]>::try_from(key).ok());
    let Some(key) = key else {
        return Config::new();
    };
    let web = pushups::WebPushConfig::new(key);
    Config::new().web(match option_env!("PUSHUPS_WEB_WORKER") {
        Some("static") => web,
        _ => web.rust_handler().service_worker_path("/sw.js"),
    })
}

#[cfg(not(target_arch = "wasm32"))]
/// Registers, asks for permission, and runs the event loop until the app exits.
pub(crate) fn run(event_loop: EventLoop) -> Result<(), Box<dyn std::error::Error>> {
    let proxy = event_loop.create_proxy();
    let (sender, events) = channel();
    pushups::set_handler(move |event| {
        // A closed channel means the app is exiting.
        let _ = sender.send(event);
        proxy.wake_up();
    });
    if let Err(error) = pushups::register() {
        log(&format!("register failed: {error}"));
    }
    std::thread::spawn(|| {
        let answer = futures_executor::block_on(pushups::request_permission());
        log(&format!("permission {answer:?}"));
    });
    Ok(event_loop.run_app(App {
        window: None,
        events,
    })?)
}

/// Registers, asks for permission, and runs the event loop, which on the web does not block and keeps running in the page's event loop after `main` returns.
#[cfg(target_arch = "wasm32")]
pub(crate) fn run(event_loop: EventLoop) -> Result<(), Box<dyn std::error::Error>> {
    let proxy = event_loop.create_proxy();
    let (sender, events) = channel();
    pushups::set_handler(move |event| {
        // A closed channel means the app is exiting.
        let _ = sender.send(event);
        proxy.wake_up();
    });
    if let Err(error) = pushups::register() {
        log(&format!("register failed: {error}"));
    }
    wasm_bindgen_futures::spawn_local(async move {
        let answer = pushups::request_permission().await;
        log(&format!("permission {answer:?}"));
    });
    Ok(event_loop.run_app(App {
        window: None,
        events,
    })?)
}

struct App {
    window: Option<Box<dyn Window>>,
    events: Receiver<Event>,
}

impl ApplicationHandler for App {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        let attributes = WindowAttributes::default().with_title("pushups winit example");
        match event_loop.create_window(attributes) {
            Ok(window) => self.window = Some(window),
            Err(error) => log(&format!("window failed: {error}")),
        }
        log(&format!("ui ready at_ms={}", now_ms()));
    }

    fn proxy_wake_up(&mut self, _event_loop: &dyn ActiveEventLoop) {
        for event in self.events.try_iter() {
            log(&describe(&event));
        }
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        if event == WindowEvent::CloseRequested {
            event_loop.exit();
        }
    }
}

fn describe(event: &Event) -> String {
    let at = now_ms();
    match event {
        Event::Token(pushups::Token::Apns(token)) => {
            let hex: String = token.iter().map(|byte| format!("{byte:02x}")).collect();
            format!("ui token at_ms={at} apns={hex}")
        }
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
    let entered = now_ms();
    log(&format!(
        "handler started_app={} entered_at_ms={entered} {} payload={}",
        message.started_app,
        timing(&message, entered),
        String::from_utf8_lossy(&message.payload)
    ));
}

/// Runs in the service worker for every push, and decides what the user sees.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn notification_for(message: Message) -> pushups::Notification {
    let at = now_ms();
    let line = timing(&message, at);
    log(&format!("worker push at_ms={at} {line}"));
    let data: serde_json::Value = serde_json::from_slice(&message.payload).unwrap_or_default();
    let text = |name: &str| {
        data.get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    pushups::Notification::new(text("title").unwrap_or_else(|| "pushups from Rust".to_owned()))
        .body(text("body").unwrap_or_else(|| format!("pushups winit example, {line}")))
        .navigate("/")
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
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis())
}

/// Prints the line, and appends it to `pushups-example.log` in the temporary directory, which outlives a process the system launched with nothing reading its output.
#[cfg(all(not(target_os = "android"), not(target_arch = "wasm32")))]
pub(crate) fn log(line: &str) {
    use std::io::Write;

    let line = format!("pushups-winit-example {line}");
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

/// Logs the line to the console, which `ci/web/example.mjs` reads with the page's lines.
#[cfg(target_arch = "wasm32")]
pub(crate) fn log(line: &str) {
    web_sys::console::log_1(&format!("pushups-winit-example {line}").into());
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
