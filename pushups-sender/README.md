# pushups-sender

[![CI](https://github.com/LucaCappelletti94/pushups/actions/workflows/ci.yml/badge.svg)](https://github.com/LucaCappelletti94/pushups/actions/workflows/ci.yml)
[![docs.rs](https://docs.rs/pushups-sender/badge.svg)](https://docs.rs/pushups-sender)
[![crates.io](https://img.shields.io/crates/v/pushups-sender.svg)](https://crates.io/crates/pushups-sender)
[![MSRV](https://img.shields.io/badge/MSRV-1.88-blue)](https://blog.rust-lang.org/2025/06/26/Rust-1.88.0/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](https://github.com/LucaCappelletti94/pushups/blob/main/LICENSE)

The app's server half of pushups' encrypted pushes. [`encrypt`] seals a plaintext for a [`Token`]'s keys as the RFC 8291 `aes128gcm` body, and the [`Sealed`] result carries it as the base64url field value FCM and APNs put under [`ENCRYPTED_FIELD`], the exact WNS raw body, or the raw Web Push body.

```rust
use pushups_sender::{encrypt, EncryptionKeys, Token, ENCRYPTED_FIELD};
use pushups_token::base64url;

// The keys of the RFC 8291 section 5 test vector.
let keys = EncryptionKeys {
    p256dh: base64url::decode(
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
    )
    .unwrap()
    .try_into()
    .unwrap(),
    auth: base64url::decode("BTBZMqHH6r4Tts7J_aSIgg").unwrap().try_into().unwrap(),
};
let token = Token::Fcm {
    token: "fcm-registration-token".to_owned(),
    keys: Some(keys),
};
let sealed = encrypt(&token, b"When I grow up, I want to be a watermelon").unwrap();
let entry = (ENCRYPTED_FIELD, sealed.to_base64url());
println!("{entry:?}");
```
