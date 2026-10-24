//! The file and message helpers the Linux and Windows probes share, so each probe writes the lines its CI script reads the same way.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use pushups::Message;

/// Appends `line` to `file` in `dir`, dropping it when the file cannot be opened.
pub fn append(dir: &Path, file: &str, line: &str) {
    if let Ok(mut log) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(file))
    {
        let _ = writeln!(log, "{line}");
    }
}

/// The line a probe logs for a push, its `started_app` and its payload.
#[must_use]
pub fn describe_message(message: &Message) -> String {
    format!(
        "message started_app={} {}",
        message.started_app,
        String::from_utf8_lossy(&message.payload)
    )
}

/// The file `name` beside the executable, which a process Windows starts finds without an environment.
#[must_use]
pub fn beside_exe(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

/// The WNS remote id from `PROBE_REMOTE_ID`, or from `remote_id.txt` beside the executable.
#[must_use]
pub fn remote_id() -> Option<String> {
    std::env::var("PROBE_REMOTE_ID").ok().or_else(|| {
        fs::read_to_string(beside_exe("remote_id.txt"))
            .ok()
            .map(|id| id.trim().to_owned())
    })
}
