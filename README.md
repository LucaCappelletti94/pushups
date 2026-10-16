# pushups

<img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/brand/mark.svg" alt="pushups logo" width="160">

[![CI](https://github.com/LucaCappelletti94/pushups/actions/workflows/ci.yml/badge.svg)](https://github.com/LucaCappelletti94/pushups/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/LucaCappelletti94/pushups/graph/badge.svg)](https://codecov.io/gh/LucaCappelletti94/pushups)
[![docs.rs](https://docs.rs/pushups/badge.svg)](https://docs.rs/pushups)
[![crates.io](https://img.shields.io/crates/v/pushups.svg)](https://crates.io/crates/pushups)
[![MSRV](https://img.shields.io/badge/MSRV-1.85-blue)](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](https://github.com/LucaCappelletti94/pushups/blob/main/LICENSE)

Push notifications for Rust apps through each platform's own service, [APNs](https://developer.apple.com/documentation/usernotifications/registering-your-app-with-apns) on iOS and macOS, [FCM](https://firebase.google.com/docs/cloud-messaging) or [UnifiedPush](https://unifiedpush.org/) on Android, [WNS](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/push-notifications/) on Windows, UnifiedPush on Linux and [Web Push](https://developer.mozilla.org/en-US/docs/Web/API/Push_API) in the browser. The app calls `install` in `main` before its toolkit starts, sets one handler and calls `register`. The handler gets the token for the app's server and every push, including the one that started the app.

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

| | Android | iOS | macOS | Windows | Linux | Web |
|---|---|---|---|---|---|---|
| Dioxus 0.7 | <a href="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/dioxus-android.png"><img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/dioxus-android-thumb.png" height="48" alt="Dioxus on Android"></a><br>yes. A tap into the running app restarts it unless `MainActivity` is `singleTask`, as [`examples/dioxus`](https://github.com/LucaCappelletti94/pushups/blob/main/examples/dioxus/android/AndroidManifest.xml) sets, and the page can occasionally freeze until [dioxus#5916](https://github.com/DioxusLabs/dioxus/pull/5916) | <a href="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/dioxus-ios.png"><img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/dioxus-ios-thumb.png" height="48" alt="Dioxus on iOS"></a><br>yes up to iOS 26, iOS 27 needs [dioxus#5893](https://github.com/DioxusLabs/dioxus/pull/5893) | <a href="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/dioxus-macos.png"><img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/dioxus-macos-thumb.png" height="48" alt="Dioxus on macOS"></a><br>yes | <a href="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/dioxus-windows.png"><img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/dioxus-windows-thumb.png" height="48" alt="Dioxus on Windows"></a><br>panics when its window cannot take focus, as on a locked screen, until [dioxus#5916](https://github.com/DioxusLabs/dioxus/pull/5916) | <a href="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/dioxus-linux.png"><img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/dioxus-linux-thumb.png" height="48" alt="Dioxus on Linux"></a><br>yes | <a href="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/dioxus-web.png"><img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/dioxus-web-thumb.png" height="48" alt="Dioxus on Web"></a><br>yes[^ios-web] |
| Tauri 2 | <a href="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/tauri-android.png"><img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/tauri-android-thumb.png" height="48" alt="Tauri on Android"></a><br>yes | <a href="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/tauri-ios.png"><img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/tauri-ios-thumb.png" height="48" alt="Tauri on iOS"></a><br>yes | <a href="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/tauri-macos.png"><img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/tauri-macos-thumb.png" height="48" alt="Tauri on macOS"></a><br>yes | <a href="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/tauri-windows.png"><img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/tauri-windows-thumb.png" height="48" alt="Tauri on Windows"></a><br>yes | <a href="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/tauri-linux.png"><img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/tauri-linux-thumb.png" height="48" alt="Tauri on Linux"></a><br>yes | |
| winit 0.31 | <a href="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/winit-android.png"><img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/winit-android-thumb.png" height="48" alt="winit on Android"></a><br>needs [winit#4752](https://github.com/rust-windowing/winit/pull/4752), which `examples/winit` pins | <a href="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/winit-ios.png"><img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/winit-ios-thumb.png" height="48" alt="winit on iOS"></a><br>yes | <a href="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/winit-macos.png"><img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/winit-macos-thumb.png" height="48" alt="winit on macOS"></a><br>yes | <a href="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/winit-windows.png"><img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/winit-windows-thumb.png" height="48" alt="winit on Windows"></a><br>yes | <a href="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/winit-linux.png"><img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/winit-linux-thumb.png" height="48" alt="winit on Linux"></a><br>yes | <a href="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/winit-web.png"><img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/winit-web-thumb.png" height="48" alt="winit on Web"></a><br>yes[^ios-web] |
| Leptos 0.8 | | | | | | <a href="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/leptos-web.png"><img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/leptos-web-thumb.png" height="48" alt="Leptos on Web"></a><br>yes[^ios-web] |
| Yew 0.23 | | | | | | <a href="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/yew-web.png"><img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/screenshots/yew-web-thumb.png" height="48" alt="Yew on Web"></a><br>yes[^ios-web] |

Dioxus and Tauri run on tao, so their cells cover a tao app. Without the winit fix, a fresh install's first launch can get no FCM token until the app is reopened.

[^ios-web]: On iOS only home-screen web apps get pushes, and they reach the page when it is next shown ([WebKit bug 268797](https://bugs.webkit.org/show_bug.cgi?id=268797)).

### Upstream fixes worth having

| PR | Why |
|---|---|
| [dioxus#5901](https://github.com/DioxusLabs/dioxus/pull/5901) | `dx` builds the iOS app icon from `Dioxus.toml`, the icon iOS shows on every notification |
| [dioxus#5898](https://github.com/DioxusLabs/dioxus/pull/5898) | `dx bundle --macos` fills each icon size from the closest PNG, so notifications show a sharp icon instead of one scaled from the first PNG listed |
| [dioxus#4063](https://github.com/DioxusLabs/dioxus/pull/4063), [dioxus#5693](https://github.com/DioxusLabs/dioxus/pull/5693) | `dx` sets the Android launcher icon, which FCM notifications use when the app names no notification icon |
| [dioxus#5591](https://github.com/DioxusLabs/dioxus/pull/5591) | `dx bundle` makes an IPA that Apple's validator accepts, the route from `dx` to the App Store |
| [dioxus#5900](https://github.com/DioxusLabs/dioxus/pull/5900) | `dx` renames its bundled JS when only this crate's JavaScript changes, so a web build stops serving the previous release's copy after an upgrade |
| [tao#1364](https://github.com/tauri-apps/tao/pull/1364) | tao reports the push token and pushes itself, so a tao app without this crate can receive them |

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
use pushups::{Config, VapidPublicKey, WebPushConfig};

let key = VapidPublicKey::from_base64url(
    "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
)?;
let config = Config::new().web(WebPushConfig::new(key));
# let _ = config;
# Ok::<(), pushups::Error>(())
```

`Token::web_push_subscription` gives the endpoint and keys the server sends to, and with the `serde` feature it serialises as the browser's `PushSubscription.toJSON()`, which `web-push` and `pywebpush` read. UnifiedPush tokens on Android and Linux take the same form.

## Linux

The crate speaks UnifiedPush over D-Bus to the user's distributor, such as KDE's, and a push to a closed app starts it through D-Bus activation for the background handler.

```rust
use pushups::{Config, LinuxConfig, UnifiedPushConfig, VapidPublicKey};

let key = VapidPublicKey::from_base64url(
    "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
)?;
let config = Config::new()
    .linux(LinuxConfig::new("org.example.App"))
    .unified_push(UnifiedPushConfig::new(key));
# let _ = config;
# Ok::<(), pushups::Error>(())
```

## Windows

The app's installer puts the Windows App SDK 2.5 runtime on the machine, and the app ships `Microsoft.WindowsAppRuntime.Bootstrap.dll` beside its executable. It gives the Object ID of its Entra ID app registration's service principal, and a push to a closed app starts it for the background handler.

```rust
use pushups::{Config, WnsConfig};

let config = Config::new().windows(WnsConfig::new("00000000-0000-0000-0000-000000000000"));
# let _ = config;
```
