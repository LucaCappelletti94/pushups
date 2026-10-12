# pushups

<img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/brand/mark.svg" alt="pushups logo" width="160">

[![CI](https://github.com/LucaCappelletti94/pushups/actions/workflows/ci.yml/badge.svg)](https://github.com/LucaCappelletti94/pushups/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/LucaCappelletti94/pushups/graph/badge.svg)](https://codecov.io/gh/LucaCappelletti94/pushups)
[![docs.rs](https://docs.rs/pushups/badge.svg)](https://docs.rs/pushups)
[![crates.io](https://img.shields.io/crates/v/pushups.svg)](https://crates.io/crates/pushups)
[![MSRV](https://img.shields.io/badge/MSRV-1.85-blue)](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](https://github.com/LucaCappelletti94/pushups/blob/main/LICENSE)

Not on crates.io yet, so the docs.rs and crates.io badges resolve after the first release.

Push notifications for Rust apps through each platform's own service, APNs on iOS and macOS, FCM or UnifiedPush on Android, WNS on Windows, UnifiedPush on Linux and Web Push in the browser. The app calls `install` in `main` before its toolkit starts, sets one handler and calls `register`. The handler gets the token for the app's server and every push, including the one that started the app.

```rust
use pushups::{Config, Event};

if let Err(error) = pushups::install(Config::new()) {
    eprintln!("no push on this target: {error}");
}
pushups::set_handler(|event| match event {
    Event::Token(token) => println!("send {token:?} to the server"),
    Event::RegistrationFailed(error) => eprintln!("no push token: {error}"),
    Event::Message(message) => println!("{} bytes, started the app: {}", message.payload.len(), message.started_app),
    Event::MessagesDropped => println!("fetch what was missed from the server"),
});
let _ = pushups::register();
```

The handler runs on the platform's delivery thread, so waking the toolkit is its job. The `stream` feature adds `pushups::events()`, the same events as a stream.

## Support

"yes" means proven on a device or in CI with the framework's current release. A linked PR is upstream work the cell waits on.

| | Android | iOS | macOS | Windows | Linux | Web |
|---|---|---|---|---|---|---|
| Dioxus 0.7 | yes, though a tap into the running app restarts it until [tao#1365](https://github.com/tauri-apps/tao/pull/1365) | yes up to iOS 26, iOS 27 needs [dioxus#5893](https://github.com/DioxusLabs/dioxus/pull/5893) | yes | panics when its window cannot take focus, as on a locked screen, until Dioxus takes [wry#1799](https://github.com/tauri-apps/wry/pull/1799) | yes | yes |
| Tauri 2 | yes | yes | yes | yes | yes | n/a |
| winit 0.31 | needs [winit#4752](https://github.com/rust-windowing/winit/pull/4752), which `examples/winit` pins | yes | yes | yes | yes | yes |
| Leptos 0.8 | n/a | n/a | n/a | n/a | n/a | yes |
| Yew 0.23 | n/a | n/a | n/a | n/a | n/a | yes |

The [examples](https://github.com/LucaCappelletti94/pushups/tree/main/examples) are the proofs. Dioxus and Tauri run on tao, so their cells cover a tao app. Without the winit fix, a fresh install's first launch can get no FCM token until the app is reopened.

## Android

The crate's Gradle module holds the FCM and UnifiedPush services. `dx` bundles it through the `dioxus` feature, a Tauri app adds [`tauri-plugin-pushups`](https://github.com/LucaCappelletti94/pushups/tree/main/tauri-plugin-pushups), and any other Gradle project depends on `io.github.lucacappelletti94:pushups-android` at the crate's version or includes `android/` through `cargo metadata` as [`examples/winit`](https://github.com/LucaCappelletti94/pushups/tree/main/examples/winit) does. A `UnifiedPushConfig` sends pushes through the device's UnifiedPush distributor when it has one.

```rust
use pushups::{Context, Message};

pushups::firebase_config!("tests/fixtures/google-services.json");

#[pushups::background_handler]
fn on_push(context: Context, message: Message) {
    let _ = (context, message.payload);
}
```

## iOS and macOS

The crate adds its push methods to whatever app delegate the toolkit installs. The bundle needs the push entitlement and, on iOS, the `remote-notification` background mode, which a `dx` app sets in `Dioxus.toml`. A Tauri app gets the iOS entitlement from `tauri-plugin-pushups` and names a macOS entitlements file in `tauri.conf.json`, as [`examples/tauri`](https://github.com/LucaCappelletti94/pushups/tree/main/examples/tauri) does.

```toml
[ios.entitlements]
aps-environment = "development"

[macos.entitlements]
"com.apple.developer.aps-environment" = "development"

[background]
remote-notifications = true
```

## Web

The app gives its VAPID public key and serves `pushups::SERVICE_WORKER` at `/pushups-sw.js`, which the `dioxus` feature has `dx` do, or sets `rust_handler` to run its own wasm as the worker, as [`examples/yew`](https://github.com/LucaCappelletti94/pushups/tree/main/examples/yew) and [`examples/leptos`](https://github.com/LucaCappelletti94/pushups/tree/main/examples/leptos) show. On iOS only home-screen web apps get pushes, and they reach the page when it is next shown ([WebKit bug 268797](https://bugs.webkit.org/show_bug.cgi?id=268797)).

```rust
use pushups::{Config, WebPushConfig};

let config = Config::new().web(WebPushConfig::new([4; 65]));
# let _ = config;
```

## Linux

The crate speaks UnifiedPush over D-Bus to the user's distributor, such as KDE's, and a push to a closed app starts it through D-Bus activation for the background handler.

```rust
use pushups::{Config, LinuxConfig, UnifiedPushConfig};

let config = Config::new()
    .linux(LinuxConfig::new("org.example.App"))
    .unified_push(UnifiedPushConfig::new([4; 65]));
# let _ = config;
```

## Windows

The app's installer puts the Windows App SDK 2.5 runtime on the machine, and the app ships `Microsoft.WindowsAppRuntime.Bootstrap.dll` beside its executable. It gives the Object ID of its Entra ID app registration's service principal, and a push to a closed app starts it for the background handler.

```rust
use pushups::{Config, WnsConfig};

let config = Config::new().windows(WnsConfig::new("00000000-0000-0000-0000-000000000000"));
# let _ = config;
```
