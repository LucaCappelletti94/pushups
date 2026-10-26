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

/// The platform offers the notification settings the example opens when the permission is
/// denied.
const OPEN_SETTINGS: bool = cfg!(any(
    target_os = "ios",
    target_os = "android",
    target_os = "macos",
    target_os = "windows"
));

/// Installs `pushups`, sets its handler and registers, then runs the Tauri app.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    match pushups::install(config()) {
        Ok(()) => log(&format!("installed at_ms={}", support::now_ms())),
        Err(error) => log(&format!("install failed: {error}")),
    }
    pushups::set_handler(|event| {
        log(&support::describe(&event));
        page_show(support::page::page_event(&event, support::now_ms()));
        if let Err(error) = support::register_again(&event) {
            log(&format!("register failed: {error}"));
        }
    });
    if let Err(error) = pushups::register() {
        log(&format!("register failed: {error}"));
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_pushups::init())
        .invoke_handler(tauri::generate_handler![allow, status, open_settings])
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
async fn allow() -> support::PermissionUi {
    let status = pushups::request_permission().await;
    log(&format!(
        "permission {}",
        support::permission(status.clone())
    ));
    support::permission_ui(status)
}

/// The permission the platform holds, which the page calls on load and when it comes back to
/// the foreground.
#[tauri::command]
async fn status() -> support::PermissionUi {
    let status = pushups::permission_status().await;
    log(&format!(
        "permission {}",
        support::permission(status.clone())
    ));
    support::permission_ui(status)
}

/// Opens the notification settings, fire and forget, which the page's button calls while the
/// platform refuses the permission.
#[tauri::command]
fn open_settings() -> Result<(), PageError> {
    pushups::open_notification_settings().map_err(|error| PageError(error.to_string()))
}

/// The error a page command reports, its description as the page shows.
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(transparent)]
#[error("{0}")]
struct PageError(String);

/// The page's events so far, and the page once it has loaded.
static PAGE: Mutex<Page> = Mutex::new(Page {
    events: Vec::new(),
    webview: None,
});

struct Page {
    events: Vec<support::page::PageEvent>,
    webview: Option<Webview>,
}

/// Adds the event to the page, now if it has loaded, else when it does.
fn page_show(event: support::page::PageEvent) {
    let mut page = PAGE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(webview) = &page.webview {
        show_in(webview, &json(&event));
    }
    page.events.push(event);
}

/// Applies the brand's assets to the page that just loaded, shows every event so far, and the
/// later ones as they come.
fn page_loaded(webview: Webview) {
    let mut page = PAGE.lock().unwrap_or_else(PoisonError::into_inner);
    let assets = support::page::assets(OPEN_SETTINGS);
    if let Err(error) = webview.eval(format!("pushupsInit({assets});")) {
        log(&format!("page update failed: {error}"));
    }
    for event in &page.events {
        show_in(&webview, &json(event));
    }
    page.webview = Some(webview);
}

/// Evaluates the page's event function with the event's JSON.
fn show_in(webview: &Webview, json: &str) {
    if let Err(error) = webview.eval(format!("pushupsEvent({json})")) {
        log(&format!("page update failed: {error}"));
    }
}

fn json(event: &support::page::PageEvent) -> String {
    serde_json::to_string(event).expect("the page's fields serialise")
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
