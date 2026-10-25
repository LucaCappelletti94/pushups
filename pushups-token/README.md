# pushups-token

[![CI](https://github.com/LucaCappelletti94/pushups/actions/workflows/ci.yml/badge.svg)](https://github.com/LucaCappelletti94/pushups/actions/workflows/ci.yml)
[![docs.rs](https://docs.rs/pushups-token/badge.svg)](https://docs.rs/pushups-token)
[![crates.io](https://img.shields.io/crates/v/pushups-token.svg)](https://crates.io/crates/pushups-token)
[![MSRV](https://img.shields.io/badge/MSRV-1.88-blue)](https://blog.rust-lang.org/2025/06/26/Rust-1.88.0/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](https://github.com/LucaCappelletti94/pushups/blob/main/LICENSE)

The push token [`pushups`](https://crates.io/crates/pushups) reports to an app and the app's server sends pushes to, in a crate of its own so the server reads it without compiling the app's push backends. [`pushups-sender`](https://crates.io/crates/pushups-sender) encrypts for it.

With the `serde` feature a token travels as JSON, tagged by its transport, bytes as unpadded base64url and times as milliseconds since the Unix epoch.

```rust
use pushups_token::{EncryptionKeys, Token};

let token = Token::Fcm {
    token: "fcm-registration-token".to_owned(),
    keys: Some(EncryptionKeys { p256dh: [4; 65], auth: [7; 16] }),
};
# #[cfg(feature = "serde")]
# {
let json = serde_json::to_string(&token).unwrap();
assert!(json.starts_with(r#"{"transport":"fcm","token":"fcm-registration-token","keys":{"p256dh":"BAQE"#));
assert_eq!(serde_json::from_str::<Token>(&json).unwrap(), token);
# }
```
