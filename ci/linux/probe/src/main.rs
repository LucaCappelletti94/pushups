//! The Linux app the end-to-end test drives inside the UnifiedPush test bed.
//!
//! It installs `pushups`, registers, and appends every event to `events.log` and every
//! background-handler call to `handled.log` in `$PROBE_OUT`, one line each. The background handler
//! also shows a notification whose tap hands back `tap:` and the payload. A token also lands in
//! `subscription.json`, the shape the `send` binary and browsers use. It runs until
//! `$PROBE_OUT/stop` appears or five minutes pass. When the session bus starts it for a push or a
//! tap, `pushups::install` keeps it headless and never returns.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use pushups::{
    Config, Event, LinuxConfig, Message, Notification, UnifiedPushConfig, VapidPublicKey,
};

const APP_ID: &str = "rs.pushups.LinuxProbe";
const RUN_BOUND: Duration = Duration::from_secs(300);

fn out_dir() -> PathBuf {
    std::env::var_os("PROBE_OUT").map_or_else(|| PathBuf::from("/tmp/probe"), PathBuf::from)
}

fn append(file: &str, line: &str) {
    let path = out_dir().join(file);
    if let Ok(mut log) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(log, "{line}");
    }
}

fn describe_message(message: &Message) -> String {
    format!(
        "message started_app={} {}",
        message.started_app,
        String::from_utf8_lossy(&message.payload)
    )
}

#[pushups::background_handler]
fn on_push(_context: pushups::Context, message: Message) {
    append("handled.log", &describe_message(&message));
    let payload = String::from_utf8_lossy(&message.payload).into_owned();
    let shown = Notification::new("pushups probe")
        .body(payload.clone())
        .data(format!("tap:{payload}"))
        .show();
    append("shown.log", &format!("shown {shown:?} {payload}"));
}

fn on_event(event: Event) {
    match event {
        Event::Token(token) => {
            if let Some(subscription) = token.web_push_subscription() {
                let json = serde_json::to_string(&subscription).expect("a subscription serialises");
                let partial = out_dir().join("subscription.json.part");
                if fs::write(&partial, json).is_ok() {
                    let _ = fs::rename(partial, out_dir().join("subscription.json"));
                }
                append("events.log", &format!("token {}", subscription.endpoint));
            } else {
                append("events.log", &format!("token {token:?}"));
            }
        }
        Event::Message(message) => append("events.log", &describe_message(&message)),
        Event::RegistrationFailed(error) => append("events.log", &format!("failed {error}")),
        other => append("events.log", &format!("{other:?}")),
    }
}

fn main() -> ExitCode {
    let _ = fs::create_dir_all(out_dir());
    let vapid = match VapidPublicKey::from_base64url(
        &std::env::var("PROBE_VAPID_B64URL").unwrap_or_default(),
    ) {
        Ok(vapid) => vapid,
        Err(error) => {
            append("events.log", &format!("PROBE_VAPID_B64URL: {error}"));
            return ExitCode::FAILURE;
        }
    };
    let config = Config::new()
        .linux(LinuxConfig::new(APP_ID))
        .unified_push(UnifiedPushConfig::new(vapid));
    if let Err(error) = pushups::install(config) {
        append("events.log", &format!("install failed {error}"));
        return ExitCode::FAILURE;
    }
    append("events.log", "ui started");
    pushups::set_handler(on_event);
    if let Err(error) = pushups::register() {
        append("events.log", &format!("register failed {error}"));
    }
    let stop = out_dir().join("stop");
    let deadline = Instant::now() + RUN_BOUND;
    while !stop.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
    }
    append("events.log", "ui stopped");
    ExitCode::SUCCESS
}
