//! The consumer transform linked into the native completion harness and iOS extension.

use pushups::{Error, Message, Notification, RenderingContext};

#[pushups::notification_service]
fn render(_context: RenderingContext, message: Message) -> Result<Notification, Error> {
    let payload: serde_json::Value = serde_json::from_slice(&message.payload)
        .map_err(|error| Error::Platform(error.to_string()))?;
    match payload["kind"].as_str().unwrap_or_default() {
        "error" => Err(Error::Platform("an error".to_owned())),
        "panic" => panic!("a panic"),
        "icon" => Ok(Notification::new("Icon title").icon("https://example.com/icon.png")),
        "clear" => Ok(Notification::new("").tag("").data("").navigate("")),
        "minimal" => Ok(Notification::new("Minimal")),
        _ => {
            if let Some(sleep) = payload["sleep"].as_u64() {
                std::thread::sleep(std::time::Duration::from_millis(sleep));
            }
            Ok(Notification::new("Rendered title")
                .body("Decrypted body")
                .tag("thread-1")
                .data(payload["original"].as_str().unwrap_or("tap-data"))
                .navigate("https://example.com"))
        }
    }
}
