//! The background handler, which runs in the process the push woke, before any UI exists.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use pushups::{Context, Message};
use pushups_example_support as support;

use super::log;

/// Where the background handler sends its request, forwarded to the host by
/// `adb reverse tcp:8787 tcp:8787`.
const CATCH_UP: &str = "127.0.0.1:8787";

/// Runs in the process the push woke, before any UI exists.
#[pushups::background_handler]
fn on_push(_context: Context, message: Message) {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    if let Err(error) = support::popup(&message) {
        log(&format!("popup failed: {error}"));
    }
    let entered = support::now_ms();
    let started = Instant::now();
    let fetched = catch_up();
    let fetch_ms = started.elapsed().as_millis();
    log(&format!(
        "handler started_app={} entered_at_ms={entered} {} fetch_ms={fetch_ms} fetch={fetched}",
        message.started_app,
        support::timing(&message, entered),
    ));
}

/// One HTTP request to the host, standing in for an app fetching what the push announced.
fn catch_up() -> String {
    let attempt = || -> std::io::Result<String> {
        let mut stream = TcpStream::connect(CATCH_UP)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.write_all(b"GET /catch-up HTTP/1.0\r\nHost: pushups\r\n\r\n")?;
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        Ok(response
            .lines()
            .next()
            .unwrap_or("empty response")
            .to_owned())
    };
    attempt().unwrap_or_else(|error| format!("failed: {error}"))
}
