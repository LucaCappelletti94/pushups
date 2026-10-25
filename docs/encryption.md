# End-to-end encrypted pushes

With the `encryption` feature, pushups creates a P-256 key pair and auth secret on the device, keeps them in the platform key store, and reports their public half in the token's `keys`. The app's server encrypts each push for those keys with RFC 8291 (`aes128gcm`, the Web Push encryption), so Google, Apple and Microsoft relay ciphertext only. pushups decrypts before any callback runs, and `Message.payload` is exactly the plaintext the server encrypted, in whatever serialisation the app chose.

| Platform | Key store |
|---|---|
| Android | Android Keystore through `android-native-keyring-store` |
| iOS, macOS | the data protection keychain, readable after first unlock, never synced |
| Windows | Credential Manager |
| Linux, web | the UnifiedPush and Web Push keys pushups already uses |

## On the server

The app sends its token to the server as JSON, with the `serde` feature on both sides. [`pushups-sender`](https://crates.io/crates/pushups-sender) reads it and encrypts.

```rust
let token: pushups_sender::Token = serde_json::from_str(&json)?;
let sealed = pushups_sender::encrypt(&token, b"any bytes the app understands")?;
```

The ciphertext travels where the transport allows it.

| Transport | Where `sealed` goes |
|---|---|
| FCM | data message key `pushups`, the value `sealed.to_base64url()` |
| APNs | top-level payload key `pushups`, the value `sealed.to_base64url()`, beside an `aps` alert with `mutable-content: 1` that shows if the device cannot decrypt |
| WNS | the raw push body `sealed.wns_body()` |
| Web Push, UnifiedPush | the request body `sealed.as_bytes()`, with `Content-Encoding: aes128gcm` |

A server in another language encrypts with any RFC 8291 implementation for the token's `p256dh` and `auth` and places the result the same way, base64url without padding. A push without the `pushups` field reaches the app as it did before.

## On the device

Android, Windows and Linux need nothing beyond the feature. On iOS the app and its notification service extension share the keys through a keychain access group. Both targets carry the same `keychain-access-groups` entitlement, and both `Info.plist` files name it under `PushupsKeychainAccessGroup`.

A push that fails to decrypt never reaches the app's callbacks. On Android, macOS, Windows and in the iOS app it is dropped with a log line. In the iOS extension the render settles `RenderingOutcome::Failed`, and the server's `aps` alert shows.

A registration that cannot read or create the keys reports `Event::RegistrationFailed`, so the server never receives a token it would have to send plaintext to. `pushups::rotate_keys()` replaces the keys and registers again, and the new keys arrive with the next `Event::Token`. Pushes sealed for the old keys fail from then on.
