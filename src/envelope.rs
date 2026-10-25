//! The encrypted push envelope the transports carry, and its opening with the device's keys.

use pushups_token::base64url;
use pushups_token::{ENCRYPTED_FIELD, EncryptionKeys};

use crate::error::Error;
use crate::key_store::KeyStore;
use crate::webpush_crypto::{DecryptError, Keys};

/// A failure to open an encrypted push, for the platform's log lines.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum OpenError {
    /// The field is not unpadded base64url.
    #[error("the field is not unpadded base64url")]
    NotBase64url,
    /// The device has no stored keys.
    #[error("the device has no stored keys")]
    NoKeys,
    /// The key store failed.
    #[error("the key store failed: {0}")]
    Store(Error),
    /// The body failed to decrypt.
    #[error("decryption failed: {0}")]
    Decrypt(DecryptError),
}

/// Opens one encrypted push, `sealed` the base64url field the transport carried.
///
/// The keys come from `store`.
///
/// # Errors
///
/// [`OpenError::NotBase64url`] when `sealed` is not unpadded base64url, [`OpenError::NoKeys`]
/// when the store holds no keys, [`OpenError::Store`] when the key store fails, and
/// [`OpenError::Decrypt`] when the body fails to decrypt.
#[cfg_attr(
    all(
        not(test),
        any(
            target_os = "android",
            target_os = "ios",
            target_os = "macos",
            target_os = "windows"
        )
    ),
    expect(dead_code, reason = "the platform ingress calls this")
)]
pub(crate) fn open(store: &KeyStore, sealed: &str) -> Result<Vec<u8>, OpenError> {
    let body = base64url::decode(sealed).ok_or(OpenError::NotBase64url)?;
    let keys = store.load().map_err(OpenError::Store)?;
    let Some(keys) = keys else {
        return Err(OpenError::NoKeys);
    };
    keys.decrypt(&body).map_err(OpenError::Decrypt)
}

/// The value of the `pushups` field when `body` is exactly the WNS envelope,
/// `{"pushups":"<base64url>"}` with no whitespace, or none for anything else.
#[must_use]
#[cfg_attr(
    all(
        not(test),
        any(
            target_os = "android",
            target_os = "ios",
            target_os = "macos",
            target_os = "windows"
        )
    ),
    expect(dead_code, reason = "the Windows `deliver` calls this")
)]
pub(crate) fn wns_sealed(body: &[u8]) -> Option<&str> {
    let prefix = format!("{{\"{ENCRYPTED_FIELD}\":\"");
    let prefix = prefix.as_bytes();
    let suffix = b"\"}";
    if body.len() < prefix.len() + suffix.len() + 1 {
        return None;
    }
    if &body[..prefix.len()] != prefix || &body[body.len() - suffix.len()..] != suffix {
        return None;
    }
    let value = &body[prefix.len()..body.len() - suffix.len()];
    if value
        .iter()
        .any(|octet| base64url::sextet(*octet).is_none())
    {
        return None;
    }
    Some(core::str::from_utf8(value).expect("base64url characters are ASCII"))
}

/// The public keys `keys` decrypts its pushes with, the token's `keys`.
#[must_use]
#[cfg_attr(
    all(
        not(test),
        any(
            target_os = "android",
            target_os = "ios",
            target_os = "macos",
            target_os = "windows"
        )
    ),
    expect(dead_code, reason = "the platform `register` calls this")
)]
pub(crate) fn token_keys(keys: &Keys) -> EncryptionKeys {
    EncryptionKeys {
        p256dh: keys.public_key(),
        auth: keys.auth(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use keyring_core::mock::Store;
    use pushups_sender::{Token, encrypt};

    use super::*;

    fn key_store() -> KeyStore {
        let store: Arc<Store> = Store::new().expect("the mock store builds");
        KeyStore::new(store, "rs.pushups")
    }

    #[test]
    fn opens_a_body_sealed_for_the_stored_keys() {
        let store = key_store();
        let keys = store.load_or_create().expect("fresh keys store");
        let token = Token::Fcm {
            token: "token".to_owned(),
            keys: Some(token_keys(&keys)),
        };
        let sealed = encrypt(&token, b"the plaintext").expect("the token's keys seal");
        let opened = open(&store, &sealed.to_base64url()).expect("the body opens");
        assert_eq!(opened, b"the plaintext");
    }

    #[test]
    fn a_tampered_body_fails_authentication() {
        let store = key_store();
        let keys = store.load_or_create().expect("fresh keys store");
        let token = Token::Fcm {
            token: "token".to_owned(),
            keys: Some(token_keys(&keys)),
        };
        let sealed = encrypt(&token, b"the plaintext").expect("the token's keys seal");
        let mut body = sealed.as_bytes().to_vec();
        let last = body.len() - 1;
        body[last] ^= 1;
        let opened = open(&store, &base64url::encode(&body));
        assert!(
            matches!(
                opened,
                Err(OpenError::Decrypt(DecryptError::AuthenticationFailure))
            ),
            "{opened:?}"
        );
    }

    #[test]
    fn an_empty_store_has_no_keys() {
        let store = key_store();
        let opened = open(&store, "YWJj");
        assert!(matches!(opened, Err(OpenError::NoKeys)));
    }

    #[test]
    fn a_non_base64url_field_does_not_open() {
        let store = key_store();
        let opened = open(&store, "not base64url!");
        assert!(matches!(opened, Err(OpenError::NotBase64url)));
    }

    #[test]
    fn wns_sealed_recognises_the_envelope() {
        let store = key_store();
        let keys = store.load_or_create().expect("fresh keys store");
        let token = Token::Fcm {
            token: "token".to_owned(),
            keys: Some(token_keys(&keys)),
        };
        let sealed = encrypt(&token, b"the plaintext").expect("the token's keys seal");
        let body = sealed.wns_body();
        let value = wns_sealed(body.as_bytes()).expect("the envelope is recognised");
        assert_eq!(value, sealed.to_base64url());
        // Whitespace inside the body.
        assert!(wns_sealed(br#"{"pushups": "abc"}"#).is_none());
        // An extra member.
        assert!(wns_sealed(br#"{"pushups":"abc","aps":{"alert":"x"}}"#).is_none());
        // A different key.
        assert!(wns_sealed(br#"{"alert":"abc"}"#).is_none());
        // Characters outside base64url.
        assert!(wns_sealed(br#"{"pushups":"ab=c"}"#).is_none());
        assert!(wns_sealed(br#"{"pushups":"ab c"}"#).is_none());
        // Not an object at all.
        assert!(wns_sealed(b"plain body").is_none());
    }
}
