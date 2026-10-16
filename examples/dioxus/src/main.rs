//! Receives pushes through `pushups` and logs every step with its wall-clock time, under the
//! logcat tag `pushups-example` on Android and in the console on the web, so a device proof can
//! time send to delivery.
//!
//! `PUSHUPS_VAPID_PUBLIC_KEY` (base64url) at build time sets the app server's VAPID key, for Web Push
//! on the web and UnifiedPush on Android and Linux. `PUSHUPS_WEB_WORKER=static` registers the static
//! worker in place of the Rust handler. `PUSHUPS_WNS_REMOTE_ID` sets the Windows remote id.

mod app;
mod background;

use pushups::{
    Config, LinuxConfig, Message, Notification, UnifiedPushConfig, VapidPublicKey, WebPushConfig,
    WnsConfig,
};
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
    let Some(key) = option_env!("PUSHUPS_VAPID_PUBLIC_KEY") else {
        return config;
    };
    let key = VapidPublicKey::from_base64url(key).expect("PUSHUPS_VAPID_PUBLIC_KEY is a VAPID key");
    let web = WebPushConfig::new(key);
    config
        .unified_push(UnifiedPushConfig::new(key))
        .web(match option_env!("PUSHUPS_WEB_WORKER") {
            Some("static") => web,
            _ => web.rust_handler(),
        })
}

/// Runs in the service worker for every push, and decides what the user sees.
async fn notification_for(message: Message) -> Notification {
    let at = support::now_ms();
    let line = support::timing(&message, at);
    log(&format!("worker push at_ms={at} {line}"));
    Notification::new(
        support::field(&message, "title").unwrap_or_else(|| "pushups from Rust".to_owned()),
    )
    .body(
        support::field(&message, "body")
            .unwrap_or_else(|| format!("built in the service worker, {line}")),
    )
    .navigate("/")
}

fn log(line: &str) {
    support::log("pushups-example", line);
}
