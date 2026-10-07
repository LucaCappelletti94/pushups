# pushups

<img src="https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/brand/mark.svg" alt="pushups logo" width="160">

[![CI](https://github.com/LucaCappelletti94/pushups/actions/workflows/ci.yml/badge.svg)](https://github.com/LucaCappelletti94/pushups/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/LucaCappelletti94/pushups/graph/badge.svg)](https://codecov.io/gh/LucaCappelletti94/pushups)
[![MSRV](https://img.shields.io/badge/MSRV-1.85-blue)](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](https://github.com/LucaCappelletti94/pushups/blob/main/LICENSE)

Push notifications in Rust, one rep per platform. APNs on iOS and macOS, FCM on Android, WNS on Windows, UnifiedPush on Linux and Web Push in the browser, behind one API that works with tao, winit or any other toolkit.

The app installs the crate before its toolkit's event loop starts, sets one handler, and asks for a token. The handler receives the token to send to the app's server, and every push, including the one that started the app before the handler existed.

```rust
use pushups::{Config, Event};

if let Err(error) = pushups::install(Config::new()) {
    eprintln!("no push on this target: {error}");
}
pushups::set_handler(|event| match event {
    Event::Token(token) => println!("send {token:?} to the server"),
    Event::RegistrationFailed(error) => eprintln!("no push token: {error}"),
    Event::Message(message) => println!(
        "{} payload bytes, started the app: {}",
        message.payload.len(),
        message.started_app,
    ),
    Event::MessagesDropped => println!("fetch what was missed from the server"),
});
let _ = pushups::register();
```

The handler runs on the thread the platform delivers on, so waking the toolkit's event loop is its job, through a tao or winit `EventLoopProxy` or a Dioxus signal. The `stream` feature adds `pushups::events()`, the same events as a `futures` stream. `pushups::request_permission().await` asks the user to let pushes show notifications.

Tokens come in the form push servers take. An APNs token becomes the hex string `a2` sends to, and a Web Push subscription becomes the base64url keys of the `web-push` crate's `SubscriptionInfo`.

```rust
use pushups::Token;

let apns = Token::Apns(vec![0xde, 0xad, 0xbe, 0xef]);
assert_eq!(apns.apns_device_token().as_deref(), Some("deadbeef"));

let web = Token::WebPush {
    endpoint: "https://push.example.net/subscription".to_owned(),
    p256dh: [4; 65],
    auth: [7; 16],
    expires: None,
};
let keys = web.web_push_keys().unwrap();
assert_eq!(keys.auth, "BwcHBwcHBwcHBwcHBwcHBw");
```

## iOS and macOS

`install` runs in `main`, on the main thread, before the toolkit starts. The crate adds its push methods to whatever app delegate the toolkit installs, and installs its own when there is none, so tao and winit need no change. It also handles the notification center's delegate, so a push arriving while the app is in front shows as a banner and a tap on a notification reaches the handler. Every push reaches the handler once, including one whose notification is tapped later.

The app bundle needs the push entitlement and, on iOS, the `remote-notification` background mode. A `dx` app sets both in `Dioxus.toml`.

```toml
[ios.entitlements]
aps-environment = "development"

[macos.entitlements]
"com.apple.developer.aps-environment" = "development"

[background]
remote-notifications = true
```

A push that wakes the app in the background runs the background handler below off the main thread, and the system gets its answer when the handler returns, or after 25 seconds, within the 30 seconds Apple allows. The push also waits on disk for the app's handler, as on Android.

## Android

The crate ships a Gradle module in `android/` holding the FCM and UnifiedPush services. A `dx` app bundles it with `#[manganis::ffi("<path to the pushups crate>/android")]` on an `extern "Kotlin" { pub type Pushups; }` block, the path relative to the app's manifest directory, as `examples/dioxus` does. A Tauri app includes it as a Gradle project. cargo-apk and xbuild have no way to bundle it yet.

The app compiles its Firebase configuration in, from the `google-services.json` the Firebase console gives, with no Google services Gradle plugin. A process the push started with no UI can run Rust at once through a background handler. The push also waits on disk for the app's handler, which gets it when the UI opens.

```rust
use pushups::{Context, Message};

pushups::firebase_config!("tests/fixtures/google-services.json");

#[pushups::background_handler]
fn on_push(context: Context, message: Message) {
    #[cfg(target_os = "android")]
    let _ = (context.android_vm(), context.android_context());
    let _ = (context, message.payload);
}
```

The payload is the FCM `data` map as a JSON object. A notification message that FCM shows itself reaches the app when the user taps it.

An app whose `Config` has a `UnifiedPushConfig` gets its pushes through a [UnifiedPush](https://unifiedpush.org) distributor, such as ntfy, when the device has one, and through FCM otherwise. The first `register` with several distributors installed asks the user to pick one. The app holds one token at a time, so on UnifiedPush it is a Web Push subscription, and the payload is the message body the server sent.

## Web

The web target needs the app's VAPID public key. Every push shows a notification, as browsers require, and a payload in the [Declarative Web Push](https://webkit.org/blog/16535/meet-declarative-web-push/) shape (`"web_push": 8030`) is shown as it is. Pushes that arrive with no page open wait in `IndexedDB` for the next page's handler.

By default the app serves `js/pushups-sw.js` from this crate at `/pushups-sw.js`, a service worker with no wasm in it. An app that needs Rust to build the notification, to decrypt an end-to-end encrypted payload for instance, sets `rust_handler`. Its own wasm then runs as the service worker, with no second build, and its `main` serves the handler there.

```rust
use pushups::{Config, Message, Notification, WebPushConfig};

async fn notification_for(message: Message) -> Notification {
    Notification::new("New message")
        .body(format!("{} bytes", message.payload.len()))
        .navigate("/inbox")
}

fn main() {
    if pushups::in_service_worker() {
        pushups::serve_service_worker(notification_for).expect("in the service worker");
        return;
    }
    let vapid_public_key = [4; 65];
    let config = Config::new().web(WebPushConfig::new(vapid_public_key).rust_handler());
    if let Err(error) = pushups::install(config) {
        eprintln!("no push on this target: {error}");
    }
}
```

Web Push needs HTTPS, or `localhost` while developing. On iOS it works only in web apps added to the home screen, and there a push reaches the handler when the app is next shown or started, even if it is open when the push arrives, since iOS gives the service worker no way to reach the open page ([WebKit bug 268797](https://bugs.webkit.org/show_bug.cgi?id=268797)). Safari never fires `pushsubscriptionchange`, so the page should call `register` on every load, which also moves a returning user to the new VAPID key after the app changes it. Webviews expose no Push API, so an app in a webview uses its platform's native backend.

## Linux

Linux has no push service of its own, so the crate speaks [UnifiedPush](https://unifiedpush.org) over the D-Bus session bus, through whichever distributor the user runs, such as KDE's. The app gives its reverse-DNS id, its name on the bus, and the VAPID public key of its server. The token is a Web Push subscription, so the server sends with any Web Push library.

```rust
use pushups::{Config, LinuxConfig, UnifiedPushConfig};

let vapid_public_key = [4; 65];
let config = Config::new()
    .linux(LinuxConfig::new("org.example.App"))
    .unified_push(UnifiedPushConfig::new(vapid_public_key));
# let _ = config;
```

`install` writes a D-Bus activation file to `~/.local/share/dbus-1/services`, unless the system already ships one for the app, so a push to a closed app starts it. A process started that way never returns from `install`. It hands the push to the background handler, keeps it on disk for the next window's handler, and exits once idle, so the app's window never opens for a push. There is no permission prompt, and `request_permission` answers whether a distributor runs. `register` returns at once, and a distributor that is offline, as KDE's is when the machine has no network, answers once it reconnects, so the token can come much later.

| Platform | Push service | Backend |
|---|---|---|
| Android | FCM or UnifiedPush | available |
| iOS, macOS | APNs | in development |
| Linux | UnifiedPush over D-Bus | in development |
| Windows | WNS | in development |
| Web | Web Push | available |
