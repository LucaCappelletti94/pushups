# tauri-plugin-pushups

Puts what [`pushups`](https://crates.io/crates/pushups) needs outside Rust into a Tauri 2 app. On Android its build adds the crate's Gradle module, holding the FCM and UnifiedPush services, to the app's Gradle project, and names the app's library for it. On iOS it adds the `aps-environment` entitlement, `development` in a debug build and `production` in a release build. A macOS app adds `com.apple.developer.aps-environment` to its own entitlements.

The app adds the plugin to its builder and uses `pushups` from Rust as any other app does, calling `install` before the builder runs, then `set_handler` and `register`.

```rust
fn with_push<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    if let Err(error) = pushups::install(pushups::Config::new()) {
        eprintln!("no push on this target: {error}");
    }
    builder.plugin(tauri_plugin_pushups::init())
}
```
