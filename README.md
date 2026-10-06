# pushups

[![CI](https://github.com/LucaCappelletti94/pushups/actions/workflows/ci.yml/badge.svg)](https://github.com/LucaCappelletti94/pushups/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/LucaCappelletti94/pushups/graph/badge.svg)](https://codecov.io/gh/LucaCappelletti94/pushups)
[![MSRV](https://img.shields.io/badge/MSRV-1.85-blue)](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](https://github.com/LucaCappelletti94/pushups/blob/main/LICENSE)

Push notifications in Rust, one rep per platform. APNs on iOS and macOS, FCM on Android, WNS on Windows and Web Push in the browser, behind one API that works with tao, winit or any other toolkit.

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

## Android

The crate ships a Gradle module in `android/` holding the FCM service. A `dx` app bundles it with `#[manganis::ffi("<path to the pushups crate>/android")]` on an `extern "Kotlin" { pub type Pushups; }` block, the path relative to the app's manifest directory, as `examples/dioxus` does. A Tauri app includes it as a Gradle project. cargo-apk and xbuild have no way to bundle it yet.

The app compiles its Firebase configuration in, from the `google-services.json` the Firebase console gives, with no Google services Gradle plugin. A process the push started with no UI can run Rust at once through a background handler. The push also waits on disk for the app's handler, which gets it when the UI opens.

```rust
use pushups::{AndroidContext, Message};

pushups::firebase_config!("tests/fixtures/google-services.json");

#[pushups::background_handler]
fn on_push(context: AndroidContext, message: Message) {
    let _ = (context.vm(), context.context(), message.payload);
}
```

The payload is the FCM `data` map as a JSON object. A notification message that FCM shows itself reaches the app when the user taps it.

| Platform | Push service | Backend |
|---|---|---|
| Android | FCM | available |
| iOS, macOS | APNs | in development |
| Windows | WNS | in development |
| Web | Web Push | in development |
