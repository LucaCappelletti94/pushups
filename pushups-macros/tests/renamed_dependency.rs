//! Both macros expand in a crate that depends on `pushups` under another name, as Cargo's
//! `package` key allows.

use std::path::Path;
use std::process::Command;

#[test]
fn both_macros_expand_under_a_renamed_dependency() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the macro crate sits in the workspace");
    let consumer = tempfile::tempdir().expect("a directory for the consumer");
    std::fs::write(
        consumer.path().join("Cargo.toml"),
        format!(
            "[package]\nname = \"renamed-consumer\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n\
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
    std::fs::copy(
        workspace.join("tests/fixtures/google-services.json"),
        consumer.path().join("google-services.json"),
    )
    .expect("the Firebase fixture");
    std::fs::create_dir(consumer.path().join("src")).expect("the consumer's src");
    std::fs::write(
        consumer.path().join("src/lib.rs"),
        "#![forbid(unsafe_code)]\n\
         pub struct FirebaseConfig;\n\
         pub struct BackgroundHandler;\n\n\
         notifications::firebase_config!(\"google-services.json\");\n\n\
         #[notifications::background_handler]\n\
         fn handle(_context: notifications::Context, _message: notifications::Message) {}\n",
    )
    .expect("the consumer's source");
    let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args(["check", "--offline", "--quiet"])
        .current_dir(consumer.path())
        .env(
            "CARGO_TARGET_DIR",
            workspace.join("target/renamed-dependency"),
        )
        .output()
        .expect("cargo runs");
    assert!(
        output.status.success(),
        "the consumer failed to build:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
