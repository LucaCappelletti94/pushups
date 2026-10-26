//! A Tauri 2 app that receives pushes through `pushups`, with `tauri-plugin-pushups` shipping the
//! Android module and the iOS push entitlement. Every event and handler call is logged with its
//! wall-clock time, to logcat under `pushups-example` on Android, so `ci/android/example.sh` can
//! time send to delivery.

use std::sync::{Mutex, PoisonError};

use pushups::{Config, Context, Message};
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
        log(&support::describe(&event));
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
    let answer = support::permission(pushups::request_permission().await);
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
    support::desktop_config()
}

/// Runs off the main thread for every push the system delivers.
#[pushups::background_handler]
fn on_push(_context: Context, message: Message) {
    support::handle_push(&message, log);
}

/// Runs in the process the push woke, and shows the push as a desktop notification on Linux and
/// Windows, before the background handler runs.
#[cfg(any(target_os = "linux", target_os = "windows"))]
#[pushups::rendering_handler]
fn render(
    _context: pushups::RenderingContext,
    message: Message,
) -> Result<pushups::Rendering, pushups::Error> {
    Ok(support::rendered(&message).into())
}

fn log(line: &str) {
    support::log("pushups-tauri-example", line);
}
