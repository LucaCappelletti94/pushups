//! The unpackaged Windows app the W0 job drives on a CI runner.
//!
//! It installs `pushups` with `PROBE_REMOTE_ID`, or the id in `remote_id.txt` beside the exe, as the
//! WNS remote id, registers, and appends every step and event to `events.log` and every
//! background-handler call to `handled.log` in `PROBE_OUT`, or `w0-out` beside the exe, one line
//! each. The channel URI also lands in `channel.txt` for the sender. It runs until `stop` appears in
//! that directory or five minutes pass. The files beside the exe are what a process Windows starts
//! for a push finds.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use pushups::{Config, Event, Message, Token, WnsConfig};
use pushups_probe_support::{beside_exe, describe_message, remote_id};

const RUN_BOUND: Duration = Duration::from_secs(300);

fn out_dir() -> PathBuf {
    std::env::var_os("PROBE_OUT").map_or_else(|| beside_exe("w0-out"), PathBuf::from)
}

fn append(file: &str, line: &str) {
    pushups_probe_support::append(&out_dir(), file, line);
}

/// Runs for every push. In a process Windows started for the push, it also calls `register`, which
/// must find the runtime `install` is still setting up.
#[pushups::background_handler]
fn on_push(_context: pushups::Context, message: Message) {
    append("handled.log", &describe_message(&message));
    if message.started_app {
        let answer = match pushups::register() {
            Ok(()) => "ok".to_owned(),
            Err(error) => format!("failed {error:?}"),
        };
        append("handled.log", &format!("register {answer}"));
    }
}

fn on_event(event: Event) {
    match event {
        Event::Token(Token::Wns {
            channel_uri,
            expires,
        }) => {
            let days = expires
                .duration_since(SystemTime::now())
                .map_or(0.0, |left| left.as_secs_f64() / 86_400.0);
            let partial = out_dir().join("channel.txt.part");
            if fs::write(&partial, &channel_uri).is_ok() {
                let _ = fs::rename(partial, out_dir().join("channel.txt"));
            }
            append(
                "events.log",
                &format!("token {channel_uri} expires_in_days={days:.2}"),
            );
        }
        Event::Token(other) => append("events.log", &format!("token {other:?}")),
        Event::Message(message) => append("events.log", &describe_message(&message)),
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
    if let Err(error) = pushups::install(Config::new().windows(WnsConfig::new(remote_id))) {
        append("events.log", &format!("install failed {error:?}"));
        return ExitCode::FAILURE;
    }
    append("events.log", "installed");
    pushups::set_handler(on_event);
    if let Err(error) = pushups::register() {
        append("events.log", &format!("register failed {error:?}"));
        return ExitCode::FAILURE;
    }
    let stop = out_dir().join("stop");
    let deadline = Instant::now() + RUN_BOUND;
    while !stop.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
    }
    append("events.log", "stopped");
    ExitCode::SUCCESS
}
