//! Receives pushes through `pushups` and logs every step with its wall-clock time, under the
//! logcat tag `pushups-example` on Android and in the console on the web, so a device proof can
//! time send to delivery.
//!
//! `PUSHUPS_VAPID_PUBLIC_KEY` (base64url) at build time sets the app server's VAPID key, for Web Push
//! on the web and UnifiedPush on Android and Linux. `PUSHUPS_WEB_WORKER=static` registers the static
//! worker in place of the Rust handler. `PUSHUPS_WNS_REMOTE_ID` sets the Windows remote id.

mod app;
mod background;

use pushups::{Config, Message, Notification};
use pushups_example_support as support;

pushups::firebase_config!("google-services.json");

fn main() {
    if pushups::in_service_worker() {
        if let Err(error) = pushups::serve_service_worker(notification_for) {
            log(&format!("serving the service worker failed: {error}"));
        }
        return;
    }
    match pushups::install(config()) {
        Ok(()) => log(&format!("installed at_ms={}", support::now_ms())),
        Err(error) => log(&format!("install failed: {error}")),
    }
    dioxus::launch(app::App);
}

fn config() -> Config {
    let config = support::desktop_config();
    match support::web_push(None) {
        Some(web) => config.web(web),
        None => config,
    }
}

/// Runs in the service worker for every push, and decides what the user sees.
async fn notification_for(message: Message) -> Notification {
    support::worker_notification(&message, log)
}

fn log(line: &str) {
    support::log("pushups-example", line);
}
