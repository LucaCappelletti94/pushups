//! The unpackaged Windows app the W1 job drives on a CI runner.
//!
//! It installs `pushups` with `PROBE_REMOTE_ID`, or the id in `remote_id.txt` beside the exe, as the
//! WNS remote id, and with the display name and the `icon.png` beside the exe as the app
//! notification's identity. It shows a notification carrying `PROBE_DATA`, or the bytes in
//! `data.txt` beside the exe, as its `data`, and appends every step and event to `events.log` in
//! `PROBE_OUT`, or `w1-out` beside the exe, one line each. It runs until `stop` appears in that
//! directory or five minutes pass. The files beside the exe are what a process Windows starts for
//! a tap finds.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use pushups::{Config, Event, Message, Notification, WnsConfig};

const RUN_BOUND: Duration = Duration::from_secs(300);
const DISPLAY_NAME: &str = "pushups w1";
const DEFAULT_TITLE: &str = "pushups w1";
const DEFAULT_DATA: &str = "w1 tap";

/// The directory beside the exe, which a process Windows starts for a tap finds without an environment.
fn beside_exe(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

fn out_dir() -> PathBuf {
    std::env::var_os("PROBE_OUT").map_or_else(|| beside_exe("w1-out"), PathBuf::from)
}

fn remote_id() -> Option<String> {
    std::env::var("PROBE_REMOTE_ID").ok().or_else(|| {
        fs::read_to_string(beside_exe("remote_id.txt"))
            .ok()
            .map(|id| id.trim().to_owned())
    })
}

fn data() -> String {
    std::env::var("PROBE_DATA")
        .ok()
        .or_else(|| fs::read_to_string(beside_exe("data.txt")).ok())
        .unwrap_or_else(|| DEFAULT_DATA.to_owned())
}

fn append(file: &str, line: &str) {
    let path = out_dir().join(file);
    if let Ok(mut log) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(log, "{line}");
    }
}

fn describe_message(message: &Message) -> String {
    format!(
        "message started_app={} data_len={} {}",
        message.started_app,
        message.payload.len(),
        String::from_utf8_lossy(&message.payload)
    )
}

fn on_event(event: Event) {
    match event {
        Event::Message(message) => append("events.log", &describe_message(&message)),
        Event::Token(token) => append("events.log", &format!("token {token:?}")),
        Event::RegistrationFailed(error) => append("events.log", &format!("failed {error}")),
        Event::MessagesDropped => append("events.log", "messages dropped"),
    }
}

fn main() -> ExitCode {
    let _ = fs::create_dir_all(out_dir());
    let Some(remote_id) = remote_id() else {
        append(
            "events.log",
            "no PROBE_REMOTE_ID and no remote_id.txt beside the exe",
        );
        return ExitCode::FAILURE;
    };
    append("events.log", &format!("started remote_id={remote_id}"));
    let icon = beside_exe("icon.png");
    let config = Config::new().windows(
        WnsConfig::new(remote_id)
            .app_notifications(DISPLAY_NAME, icon.to_string_lossy().into_owned()),
    );
    if let Err(error) = pushups::install(config) {
        append("events.log", &format!("install failed {error:?}"));
        return ExitCode::FAILURE;
    }
    append("events.log", "installed");
    pushups::set_handler(on_event);
    let payload = data();
    let title = std::env::var("PROBE_TITLE").unwrap_or_else(|_| DEFAULT_TITLE.to_owned());
    let notification = Notification::new(title)
        .body("a tap on me delivers my data")
        .data(payload.clone())
        .icon(icon.to_string_lossy().into_owned());
    match notification.show() {
        Ok(()) => append("events.log", &format!("shown data_len={}", payload.len())),
        Err(error) => append("events.log", &format!("show failed {error:?}")),
    }
    let stop = out_dir().join("stop");
    let deadline = Instant::now() + RUN_BOUND;
    while !stop.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
    }
    append("events.log", "stopped");
    ExitCode::SUCCESS
}
