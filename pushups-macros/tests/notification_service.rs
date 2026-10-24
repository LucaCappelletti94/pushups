//! The `notification_service` macro expands in a crate that depends on `pushups` under another
//! name, and the transform's signature is type-checked through the renamed dependency.

use std::path::Path;
use std::process::Command;

#[test]
fn notification_service_expands_under_a_renamed_dependency() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the macro crate sits in the workspace");
    let consumer = tempfile::tempdir().expect("a directory for the consumer");
    std::fs::write(
        consumer.path().join("Cargo.toml"),
        format!(
            "[package]\nname = \"notification-service-consumer\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n\
             [dependencies]\nnotifications = {{ package = \"pushups\", path = {:?} }}\n\n[workspace]\n",
            workspace.display().to_string()
        ),
    )
    .expect("the consumer's manifest");
    // The workspace's lock keeps the check offline, since every dependency is already fetched.
    std::fs::copy(
        workspace.join("Cargo.lock"),
        consumer.path().join("Cargo.lock"),
    )
    .expect("the workspace's lock");
    std::fs::create_dir(consumer.path().join("src")).expect("the consumer's src");
    std::fs::write(
        consumer.path().join("src/lib.rs"),
        "#[notifications::notification_service]\n\
         fn on_render(\n    _context: notifications::RenderingContext,\n\
         message: notifications::Message,\n) -> Result<notifications::Rendering, notifications::Error> {\n\
         Ok(notifications::Notification::new(\"Rendered\").body(message.payload.len().to_string()).into())\n}\n",
    )
    .expect("the consumer's source");
    let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args(["check", "--offline", "--quiet"])
        .current_dir(consumer.path())
        .env(
            "CARGO_TARGET_DIR",
            std::env::current_exe()
                .expect("the test executable")
                .with_file_name("notification-service"),
        )
        .output()
        .expect("cargo runs");
    assert!(
        output.status.success(),
        "the consumer failed to build:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
