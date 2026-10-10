//! The Linux app the end-to-end test drives inside the UnifiedPush test bed.
//!
//! It installs `pushups`, registers, and appends every event to `events.log` and every
//! background-handler call to `handled.log` in `$PROBE_OUT`, one line each. A token also lands in
//! `subscription.json`, the shape the `send` binary and browsers use. It runs until
//! `$PROBE_OUT/stop` appears or five minutes pass. When the session bus starts it for a push,
//! `pushups::install` keeps it headless and never returns.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use pushups::{Config, Event, LinuxConfig, Message, UnifiedPushConfig};

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
}

fn on_event(event: Event) {
    match event {
        Event::Token(token) => {
            if let (pushups::Token::WebPush { endpoint, .. }, Some(keys)) =
                (&token, token.web_push_keys())
            {
                let subscription = serde_json::json!({
                    "endpoint": endpoint,
                    "keys": { "p256dh": keys.p256dh, "auth": keys.auth },
                });
                let partial = out_dir().join("subscription.json.part");
                if fs::write(&partial, subscription.to_string()).is_ok() {
                    let _ = fs::rename(partial, out_dir().join("subscription.json"));
                }
                append("events.log", &format!("token {endpoint}"));
            } else {
                append("events.log", &format!("token {token:?}"));
            }
        }
        Event::Message(message) => append("events.log", &describe_message(&message)),
        Event::RegistrationFailed(error) => append("events.log", &format!("failed {error}")),
        other => append("events.log", &format!("{other:?}")),
    }
}

fn vapid_public_key() -> Option<[u8; 65]> {
    let hex = std::env::var("PROBE_VAPID_HEX").ok()?;
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(hex.get(index..index + 2)?, 16).ok())
        .collect::<Option<_>>()?;
    bytes.try_into().ok()
}

fn main() -> ExitCode {
    let _ = fs::create_dir_all(out_dir());
    let Some(vapid) = vapid_public_key() else {
        append("events.log", "PROBE_VAPID_HEX is not a 65-byte hex key");
        return ExitCode::FAILURE;
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
