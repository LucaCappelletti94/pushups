//! The diagnostics the macros emit, through a real `cargo check` of a consumer, so the spans
//! land on the caller's tokens as the user sees them.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The line and column a diagnostic of the consumer's `src/lib.rs` must point at.
fn assert_span(stderr: &str, line: u32, column: u32) {
    let span = format!("src/lib.rs:{line}:{column}");
    assert!(stderr.contains(&span), "missing {span} in:\n{stderr}");
}

/// A consumer crate depending on `pushups` under the name `notifications`, and the target
/// directory its checks build in.
fn consumer() -> (tempfile::TempDir, PathBuf) {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the macro crate sits in the workspace");
    let consumer = tempfile::tempdir().expect("a directory for the consumer");
    std::fs::write(
        consumer.path().join("Cargo.toml"),
        format!(
            "[package]\nname = \"compile-errors-consumer\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n\
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
    (consumer, workspace.join("target/compile-errors"))
}

/// A `cargo check` of the consumer with the source in `src/lib.rs` and, when `json` is `Some`,
/// the file in `google-services.json`.
fn check(consumer: &Path, target: &Path, source: &str, json: Option<&str>) -> String {
    std::fs::create_dir(consumer.join("src")).expect("the consumer's src");
    std::fs::write(consumer.join("src/lib.rs"), source).expect("the consumer's source");
    if let Some(json) = json {
        std::fs::write(consumer.join("google-services.json"), json).expect("the Firebase file");
    }
    let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args(["check", "--offline", "--quiet"])
        .current_dir(consumer)
        .env("CARGO_TARGET_DIR", target)
        .output()
        .expect("cargo runs");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        !output.status.success(),
        "the consumer is expected to fail:\n{stderr}"
    );
    stderr
}

const FIREBASE_SOURCE: &str = "notifications::firebase_config!(\"google-services.json\");";

#[test]
fn firebase_config_missing_file_points_at_the_literal() {
    let (consumer, target) = consumer();
    let stderr = check(consumer.path(), &target, FIREBASE_SOURCE, None);
    let manifest = consumer.path().join("google-services.json");
    let prefix = format!(
        "error: firebase_config: cannot read {manifest}: ",
        manifest = manifest.display()
    );
    assert!(
        stderr.lines().any(|line| line.starts_with(&prefix)),
        "got:\n{stderr}"
    );
    assert_span(&stderr, 1, 33);
}

#[test]
fn firebase_config_invalid_json_points_at_the_literal() {
    let (consumer, target) = consumer();
    let stderr = check(consumer.path(), &target, FIREBASE_SOURCE, Some("not JSON"));
    let manifest = consumer.path().join("google-services.json");
    let prefix = format!(
        "error: firebase_config: {manifest} is not valid JSON: ",
        manifest = manifest.display()
    );
    assert!(
        stderr.lines().any(|line| line.starts_with(&prefix)),
        "got:\n{stderr}"
    );
    assert_span(&stderr, 1, 33);
}

#[test]
fn firebase_config_missing_client_points_at_the_literal() {
    let (consumer, target) = consumer();
    let stderr = check(
        consumer.path(),
        &target,
        FIREBASE_SOURCE,
        Some(r#"{"project_info": {"project_number": "1", "project_id": "demo"}}"#),
    );
    assert!(
        stderr
            .lines()
            .any(|line| line == "error: firebase_config: the file has no \"client\" array"),
        "got:\n{stderr}"
    );
    assert_span(&stderr, 1, 33);
}

#[test]
fn background_handler_arguments_point_at_the_argument() {
    let (consumer, target) = consumer();
    let stderr = check(
        consumer.path(),
        &target,
        "#[notifications::background_handler(bogus)]\nfn handle() {}",
        None,
    );
    assert!(
        stderr
            .lines()
            .any(|line| line == "error: background_handler: takes no arguments"),
        "got:\n{stderr}"
    );
    assert_span(&stderr, 1, 37);
}

#[test]
fn background_handler_async_points_at_the_name() {
    let (consumer, target) = consumer();
    let stderr = check(
        consumer.path(),
        &target,
        "#[notifications::background_handler]\nasync fn handle() {}",
        None,
    );
    assert!(
        stderr
            .lines()
            .any(|line| line
                == "error: background_handler: the handler must be a synchronous function"),
        "got:\n{stderr}"
    );
    assert_span(&stderr, 2, 10);
}

#[test]
fn background_handler_non_function_points_at_the_item() {
    let (consumer, target) = consumer();
    let stderr = check(
        consumer.path(),
        &target,
        "#[notifications::background_handler]\nstruct NotAFunction;",
        None,
    );
    assert!(
        stderr.lines().any(|line| line == "error: expected `fn`"),
        "got:\n{stderr}"
    );
    assert_span(&stderr, 2, 1);
}

#[test]
fn rendering_handler_arguments_point_at_the_argument() {
    let (consumer, target) = consumer();
    let stderr = check(
        consumer.path(),
        &target,
        "#[notifications::rendering_handler(bogus)]\nfn render() {}",
        None,
    );
    assert!(
        stderr
            .lines()
            .any(|line| line == "error: rendering_handler: takes no arguments"),
        "got:\n{stderr}"
    );
    assert_span(&stderr, 1, 36);
}

#[test]
fn rendering_handler_async_points_at_the_name() {
    let (consumer, target) = consumer();
    let stderr = check(
        consumer.path(),
        &target,
        "#[notifications::rendering_handler]\nasync fn render() {}",
        None,
    );
    assert!(
        stderr
            .lines()
            .any(|line| line
                == "error: rendering_handler: the handler must be a synchronous function"),
        "got:\n{stderr}"
    );
    assert_span(&stderr, 2, 10);
}

#[test]
fn rendering_handler_non_function_points_at_the_item() {
    let (consumer, target) = consumer();
    let stderr = check(
        consumer.path(),
        &target,
        "#[notifications::rendering_handler]\nstruct NotAFunction;",
        None,
    );
    assert!(
        stderr.lines().any(|line| line == "error: expected `fn`"),
        "got:\n{stderr}"
    );
    assert_span(&stderr, 2, 1);
}
