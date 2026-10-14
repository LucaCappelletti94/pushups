//! The app both entry points run, `main` on desktop, iOS and the web and `android_main` on Android.

use std::rc::Rc;
use std::sync::mpsc::{Receiver, channel};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

use pushups::{Config, Context, Event, Message, VapidPublicKey};

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

use crate::screen::Screen;

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
    if let Some(key) = option_env!("PUSHUPS_VAPID_PUBLIC_KEY") {
        let key = VapidPublicKey::from_base64url(key).expect("PUSHUPS_VAPID_PUBLIC_KEY is a VAPID key");
        config = config.unified_push(UnifiedPushConfig::new(key));
    }
    config
}

/// The web configuration from `PUSHUPS_VAPID_PUBLIC_KEY` (base64url). `PUSHUPS_WEB_WORKER=static` registers the static worker, which `build.rs` writes from `pushups::SERVICE_WORKER`, in place of the Rust handler, which runs from the `sw.js` entry.
#[cfg(target_arch = "wasm32")]
fn config() -> Config {
    let Some(key) = option_env!("PUSHUPS_VAPID_PUBLIC_KEY") else {
        return Config::new();
    };
    let key = VapidPublicKey::from_base64url(key).expect("PUSHUPS_VAPID_PUBLIC_KEY is a VAPID key");
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
    Ok(event_loop.run_app(App::new(events))?)
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
    Ok(event_loop.run_app(App::new(events))?)
}

struct App {
    window: Option<Rc<dyn Window>>,
    screen: Screen,
    events: Receiver<Event>,
}

impl App {
    fn new(events: Receiver<Event>) -> Self {
        Self {
            window: None,
            screen: Screen::new(),
            events,
        }
    }
}

impl ApplicationHandler for App {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        let attributes = WindowAttributes::default().with_title("pushups winit example");
        // A desktop or page window starts small, while a phone's fills the screen.
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        let attributes = attributes.with_surface_size(winit::dpi::LogicalSize::new(640.0, 400.0));
        // The page shows the canvas under its heading.
        #[cfg(target_arch = "wasm32")]
        let attributes = attributes.with_platform_attributes(Box::new(
            winit::platform::web::WindowAttributesWeb::default().with_append(true),
        ));
        match event_loop.create_window(attributes) {
            Ok(window) => {
                let window: Rc<dyn Window> = Rc::from(window);
                if let Err(error) = self.screen.attach(Rc::clone(&window)) {
                    log(&format!("drawing surface failed: {error}"));
                }
                self.window = Some(window);
            }
            Err(error) => log(&format!("window failed: {error}")),
        }
        log(&format!("ui ready at_ms={}", now_ms()));
    }

    fn destroy_surfaces(&mut self, _event_loop: &dyn ActiveEventLoop) {
        self.screen.detach();
        self.window = None;
    }

    fn proxy_wake_up(&mut self, _event_loop: &dyn ActiveEventLoop) {
        for event in self.events.try_iter() {
            log(&describe(&event));
            self.screen.push(shown(&event));
        }
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.screen.draw(),
            WindowEvent::SurfaceResized(_) => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

/// The event as the window shows it, a push as the `body` its payload carries.
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

fn describe(event: &Event) -> String {
    let at = now_ms();
    match event {
        Event::Token(token) => {
            if let Some(subscription) = token.web_push_subscription() {
                let json = serde_json::to_string(&subscription).expect("a subscription serialises");
                format!("ui token at_ms={at} subscription={json}")
            } else if let Some(hex) = token.apns_device_token() {
                format!("ui token at_ms={at} apns={hex}")
            } else {
                format!("ui token at_ms={at} {token:?}")
            }
        }
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

/// Runs in the service worker for every push, and decides what the user sees.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn notification_for(message: Message) -> pushups::Notification {
    let at = now_ms();
    let line = timing(&message, at);
    log(&format!("worker push at_ms={at} {line}"));
    pushups::Notification::new("pushups from Rust")
        .body(format!("pushups winit example, {line}"))
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
