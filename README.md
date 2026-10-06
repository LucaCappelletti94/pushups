# pushups

[![CI](https://github.com/LucaCappelletti94/pushups/actions/workflows/ci.yml/badge.svg)](https://github.com/LucaCappelletti94/pushups/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/LucaCappelletti94/pushups/graph/badge.svg)](https://codecov.io/gh/LucaCappelletti94/pushups)
[![MSRV](https://img.shields.io/badge/MSRV-1.85-blue)](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](https://github.com/LucaCappelletti94/pushups/blob/main/LICENSE)

Push notifications in Rust, one rep per platform. APNs on iOS and macOS, FCM on Android, WNS on Windows and Web Push in the browser, behind one API that works with tao, winit or any other toolkit.

The app sets one handler. It receives the token to send to the app's server, and every push, including the one that started the app before the handler existed.

```rust
use pushups::Event;

pushups::set_handler(|event| match event {
    Event::Token(token) => println!("send {token:?} to the server"),
    Event::RegistrationFailed(error) => eprintln!("no push token: {error}"),
    Event::Message(message) => println!(
        "{} payload bytes, started the app: {}",
        message.payload.len(),
        message.started_app,
    ),
});
```

The handler runs on the thread the platform delivers on, so waking the toolkit's event loop is its job, through a tao or winit `EventLoopProxy` or a Dioxus signal. The `stream` feature adds `pushups::events()`, the same events as a `futures` stream.

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

| Platform | Push service |
|---|---|
| iOS, macOS | APNs |
| Android | FCM |
| Windows | WNS |
| Web | Web Push |

The platform backends are in development.
