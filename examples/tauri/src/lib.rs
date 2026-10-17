//! A Tauri 2 app that receives pushes through `pushups`, with `tauri-plugin-pushups` shipping the
//! Android module and the iOS push entitlement. Every event and handler call is logged with its
//! wall-clock time, to logcat under `pushups-example` on Android, so `ci/android/example.sh` can
//! time send to delivery.

use std::sync::{Mutex, PoisonError};

use pushups::{
    Config, Context, Event, LinuxConfig, Message, UnifiedPushConfig, VapidPublicKey, WnsConfig,
};
use pushups_example_support as support;
use tauri::Webview;
use tauri::webview::PageLoadEvent;

#[cfg(target_os = "android")]
pushups::firebase_config!("google-services.json");

/// Installs `pushups`, sets its handler and registers, then runs the Tauri app.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    match pushups::install(config()) {
        Ok(()) => log(&format!("installed at_ms={}", support::now_ms())),
        Err(error) => log(&format!("install failed: {error}")),
    }
    pushups::set_handler(|event| {
        log(&describe(&event));
        page_show(support::shown(&event));
        if let Err(error) = support::register_again(&event) {
            log(&format!("register failed: {error}"));
        }
    });
    if let Err(error) = pushups::register() {
        log(&format!("register failed: {error}"));
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_pushups::init())
        .invoke_handler(tauri::generate_handler![allow])
        .on_page_load(|webview, payload| {
            if payload.event() == PageLoadEvent::Finished {
                page_loaded(webview.clone());
            }
        })
        .run(tauri::generate_context!())
        .expect("the Tauri app runs");
}

/// Asks for the notification permission, which the page's button calls through the `__TAURI__`
/// API since the prompt needs the click's gesture, and returns the answer for the page to show.
#[tauri::command]
async fn allow() -> String {
    let answer = match pushups::request_permission().await {
        Ok(permission) => format!("{permission:?}"),
        Err(error) => format!("failed: {error}"),
    };
    log(&format!("permission {answer}"));
    answer
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

/// The app's configuration from build-time keys, `PUSHUPS_VAPID_PUBLIC_KEY` (base64url) for UnifiedPush on Linux and Android and `PUSHUPS_WNS_REMOTE_ID` for Windows.
fn config() -> Config {
    // The bundle identifier, which also names the app on the Linux session bus.
    let mut config = Config::new().linux(
        LinuxConfig::new("rs.pushups.example")
            .app_notifications("pushups example", support::app_icon()),
    );
    if let Some(remote_id) = option_env!("PUSHUPS_WNS_REMOTE_ID") {
        config = config.windows(
            WnsConfig::new(remote_id).app_notifications("pushups example", support::app_icon()),
        );
    }
    if let Some(key) = option_env!("PUSHUPS_VAPID_PUBLIC_KEY") {
        let key =
            VapidPublicKey::from_base64url(key).expect("PUSHUPS_VAPID_PUBLIC_KEY is a VAPID key");
        config = config.unified_push(UnifiedPushConfig::new(key));
    }
    config
}

fn describe(event: &Event) -> String {
    let at = support::now_ms();
    match event {
        Event::Token(token) => format!("ui token at_ms={at} {}", support::token(token)),
        Event::RegistrationFailed(error) => format!("ui registration failed at_ms={at} {error}"),
        Event::Message(message) => format!(
            "ui message at_ms={at} started_app={} {} payload={}",
            message.started_app,
            support::timing(message, at),
            String::from_utf8_lossy(&message.payload)
        ),
        Event::MessagesDropped => format!("ui messages dropped at_ms={at}"),
    }
}

/// Runs off the main thread for every push the system delivers.
#[pushups::background_handler]
fn on_push(_context: Context, message: Message) {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    if let Err(error) = support::popup(&message) {
        log(&format!("popup failed: {error}"));
    }
    let entered = support::now_ms();
    log(&format!(
        "handler started_app={} entered_at_ms={entered} {} payload={}",
        message.started_app,
        support::timing(&message, entered),
        String::from_utf8_lossy(&message.payload)
    ));
}

fn log(line: &str) {
    support::log("pushups-tauri-example", line);
}
