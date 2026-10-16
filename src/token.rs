use std::time::SystemTime;

use crate::VapidKeyError;

/// A push token, the address the app's server sends pushes to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Token {
    /// An APNs device token. Apple gives no guarantee on its length, which is 32 bytes today.
    Apns(Vec<u8>),
    /// An FCM registration token.
    Fcm(String),
    /// A WNS channel.
    Wns {
        /// The channel URI the server posts to.
        channel_uri: String,
        /// When the channel stops accepting pushes.
        expires: SystemTime,
    },
    /// A Web Push subscription.
    WebPush {
        /// The push service URL the server posts to.
        endpoint: String,
        /// The subscription's P-256 public key, uncompressed.
        p256dh: [u8; 65],
        /// The subscription's authentication secret.
        auth: [u8; 16],
        /// When the subscription expires, if the push service set an expiry.
        expires: Option<SystemTime>,
    },
}

/// The keys of a Web Push subscription, base64url encoded without padding.
///
/// These are the `keys` of a browser `PushSubscription` serialised to JSON, and the two strings
/// `web_push::SubscriptionInfo::new` takes after the endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct WebPushKeys {
    /// The subscription's P-256 public key.
    pub p256dh: String,
    /// The subscription's authentication secret.
    pub auth: String,
}

/// A Web Push subscription, everything the app's server needs to send the app pushes.
///
/// With the `serde` feature it serialises as a browser's `PushSubscription.toJSON()`, the JSON
/// that `web-push` (Rust and npm) and `pywebpush` read.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct WebPushSubscription {
    /// The push service URL the server posts to.
    pub endpoint: String,
    /// When the subscription expires, if the push service set an expiry, serialised as
    /// milliseconds since the Unix epoch or `null`.
    #[cfg_attr(feature = "serde", serde(serialize_with = "milliseconds_since_epoch"))]
    pub expiration_time: Option<SystemTime>,
    /// The subscription's keys.
    pub keys: WebPushKeys,
}

#[cfg(feature = "serde")]
#[expect(
    clippy::ref_option,
    reason = "`serialize_with` passes the field by reference"
)]
fn milliseconds_since_epoch<S: serde::Serializer>(
    time: &Option<SystemTime>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::Error as _;
    let Some(time) = time else {
        return serializer.serialize_none();
    };
    let since_epoch = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| S::Error::custom("the expiration time is before the Unix epoch"))?;
    let milliseconds = u64::try_from(since_epoch.as_millis())
        .map_err(|_| S::Error::custom("the expiration time overflows u64 milliseconds"))?;
    serializer.serialize_some(&milliseconds)
}

impl Token {
    /// The APNs device token as lowercase hex, the form APNs request paths and `a2` take.
    #[must_use]
    pub fn apns_device_token(&self) -> Option<String> {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let Self::Apns(bytes) = self else {
            return None;
        };
        let mut hex = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            hex.push(char::from(DIGITS[usize::from(byte >> 4)]));
            hex.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
        }
        Some(hex)
    }

    /// The Web Push subscription, with its keys base64url encoded as a server expects them.
    #[must_use]
    pub fn web_push_subscription(&self) -> Option<WebPushSubscription> {
        let Self::WebPush {
            endpoint,
            p256dh,
            auth,
            expires,
        } = self
        else {
            return None;
        };
        Some(WebPushSubscription {
            endpoint: endpoint.clone(),
            expiration_time: *expires,
            keys: WebPushKeys {
                p256dh: base64url(p256dh),
                auth: base64url(auth),
            },
        })
    }
}

/// RFC 4648 base64 with the URL and filename safe alphabet and no padding.
pub(crate) fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let symbol = |index: u8| char::from(ALPHABET[usize::from(index)]);
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(symbol(first >> 2));
        encoded.push(symbol(((first & 0x03) << 4) | (second >> 4)));
        if chunk.len() > 1 {
            encoded.push(symbol(((second & 0x0f) << 2) | (third >> 6)));
        }
        if chunk.len() > 2 {
            encoded.push(symbol(third & 0x3f));
        }
    }
    encoded
}

/// Decodes base64url without padding, or none when the text is not unpadded base64url.
///
/// The unused bits of a trailing sextet may hold anything, so this discards them where the
/// reference decoders reject them.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn base64url_decode(text: &str) -> Option<Vec<u8>> {
    if text.len() % 4 == 1 {
        return None;
    }
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3);
    let mut pending = 0u16;
    let mut pending_bits = 0;
    for character in text.chars() {
        let sextet = base64url_sextet(character)?;
        pending = (pending << 6) | u16::from(sextet);
        pending_bits += 6;
        if pending_bits >= 8 {
            pending_bits -= 8;
            // The shift leaves the byte in the low eight bits.
            bytes.push((pending >> pending_bits).to_le_bytes()[0]);
            pending &= (1 << pending_bits) - 1;
        }
    }
    Some(bytes)
}

/// Decodes the 87 characters of unpadded base64url that encode a 65-byte VAPID public key.
pub(crate) fn base64url_decode_vapid_key(text: &str) -> Result<[u8; 65], VapidKeyError> {
    let mut bytes = [0u8; 65];
    let mut written = 0;
    let mut pending = 0u16;
    let mut pending_bits = 0;
    for (index, character) in text.char_indices() {
        let sextet =
            base64url_sextet(character).ok_or(VapidKeyError::Character { index, character })?;
        pending = (pending << 6) | u16::from(sextet);
        pending_bits += 6;
        if pending_bits >= 8 {
            pending_bits -= 8;
            if let Some(byte) = bytes.get_mut(written) {
                *byte = (pending >> pending_bits).to_le_bytes()[0];
            }
            written += 1;
            pending &= (1 << pending_bits) - 1;
        }
    }
    if text.len() != 87 {
        return Err(VapidKeyError::Length(text.len()));
    }
    Ok(bytes)
}

fn base64url_sextet(character: char) -> Option<u8> {
    let byte = u8::try_from(character).ok()?;
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'-' => Some(62),
        b'_' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use base64::Engine as _;

    use super::*;
    use crate::{Error, VapidKeyError, VapidPublicKey};

    const AUTH: [u8; 16] = [
        0x05, 0x30, 0x59, 0x32, 0xa1, 0xc7, 0xea, 0xbe, 0x13, 0xb6, 0xce, 0xc9, 0xfd, 0xa4, 0x88,
        0x82,
    ];
    const P256DH: [u8; 65] = [
        0x04, 0x25, 0x71, 0xb2, 0xbe, 0xcd, 0xfd, 0xe3, 0x60, 0x55, 0x1a, 0xaf, 0x1e, 0xd0, 0xf4,
        0xcd, 0x36, 0x6c, 0x11, 0xce, 0xbe, 0x55, 0x5f, 0x89, 0xbc, 0xb7, 0xb1, 0x86, 0xa5, 0x33,
        0x39, 0x17, 0x31, 0x68, 0xec, 0xe2, 0xeb, 0xe0, 0x18, 0x59, 0x7b, 0xd3, 0x04, 0x79, 0xb8,
        0x6e, 0x3c, 0x8f, 0x8e, 0xce, 0xd5, 0x77, 0xca, 0x59, 0x18, 0x7e, 0x92, 0x46, 0x99, 0x0d,
        0xb6, 0x82, 0x00, 0x8b, 0x0e,
    ];

    fn web_push(p256dh: [u8; 65], auth: [u8; 16]) -> Token {
        Token::WebPush {
            endpoint: "https://push.example.net/sub".to_owned(),
            p256dh,
            auth,
            expires: None,
        }
    }

    #[test]
    fn apns_device_token_is_lowercase_hex_of_every_byte() {
        let token = Token::Apns(vec![0x00, 0x0f, 0xa0, 0xff, 0x5c]);
        assert_eq!(token.apns_device_token().as_deref(), Some("000fa0ff5c"));
    }

    #[test]
    fn apns_device_token_is_none_for_other_transports() {
        assert_eq!(Token::Fcm("abc".to_owned()).apns_device_token(), None);
        assert_eq!(web_push(P256DH, AUTH).apns_device_token(), None);
    }

    fn keys(token: &Token) -> WebPushKeys {
        token.web_push_subscription().unwrap().keys
    }

    /// The user agent key and authentication secret of RFC 8291, section 5.
    #[test]
    fn web_push_keys_match_rfc_8291_example() {
        let keys = keys(&web_push(P256DH, AUTH));
        assert_eq!(keys.auth, "BTBZMqHH6r4Tts7J_aSIgg");
        assert_eq!(keys.p256dh, P256DH_BASE64URL);
    }

    const P256DH_BASE64URL: &str =
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";

    #[test]
    fn web_push_keys_use_the_url_alphabet_without_padding() {
        let mut auth = [0u8; 16];
        auth[..3].copy_from_slice(&[0xfb, 0xff, 0xbf]);
        let mut p256dh = [0xffu8; 65];
        p256dh[0] = 0xfb;
        let keys = keys(&web_push(p256dh, auth));
        assert_eq!(keys.auth, "-_-_AAAAAAAAAAAAAAAAAA");
        assert!(keys.p256dh.starts_with("-___"));
        assert!(keys.p256dh.ends_with("__8"));
        assert_eq!(keys.p256dh.len(), 87);
    }

    #[test]
    fn web_push_subscription_carries_the_endpoint_and_expiry() {
        let expires = SystemTime::UNIX_EPOCH + Duration::from_millis(1_791_881_200_386);
        let token = Token::WebPush {
            endpoint: "https://push.example.net/sub".to_owned(),
            p256dh: P256DH,
            auth: AUTH,
            expires: Some(expires),
        };
        let subscription = token.web_push_subscription().unwrap();
        assert_eq!(subscription.endpoint, "https://push.example.net/sub");
        assert_eq!(subscription.expiration_time, Some(expires));
        assert_eq!(
            web_push(P256DH, AUTH)
                .web_push_subscription()
                .unwrap()
                .expiration_time,
            None
        );
    }

    #[test]
    fn web_push_subscription_is_none_for_other_transports() {
        assert_eq!(Token::Apns(vec![1, 2]).web_push_subscription(), None);
        assert_eq!(Token::Fcm("abc".to_owned()).web_push_subscription(), None);
        let wns = Token::Wns {
            channel_uri: "https://wns.example.net/c".to_owned(),
            expires: SystemTime::UNIX_EPOCH,
        };
        assert_eq!(wns.web_push_subscription(), None);
    }

    /// The shape a browser's `PushSubscription.toJSON()` gives, which Web Push servers read.
    #[cfg(feature = "serde")]
    #[test]
    fn web_push_subscription_serialises_as_push_subscription_to_json() {
        let token = Token::WebPush {
            endpoint: "https://push.example.net/sub?a=\"b\"".to_owned(),
            p256dh: P256DH,
            auth: AUTH,
            expires: Some(SystemTime::UNIX_EPOCH + Duration::from_millis(1_791_881_200_386)),
        };
        let expected = serde_json::json!({
            "endpoint": "https://push.example.net/sub?a=\"b\"",
            "expirationTime": 1_791_881_200_386_u64,
            "keys": { "p256dh": P256DH_BASE64URL, "auth": "BTBZMqHH6r4Tts7J_aSIgg" },
        });
        let json = serde_json::to_value(token.web_push_subscription().unwrap()).unwrap();
        assert_eq!(json, expected);
        let json = serde_json::to_value(web_push(P256DH, AUTH).web_push_subscription()).unwrap();
        assert_eq!(json["expirationTime"], serde_json::Value::Null);
    }

    #[test]
    fn vapid_key_from_base64url_agrees_with_the_base64_crate() {
        let mut cases = vec![[0u8; 65], [0xff; 65], P256DH];
        let mut state = 1u32;
        cases.extend((0..64).map(|_| {
            std::array::from_fn(|_| {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                state.to_be_bytes()[0]
            })
        }));
        for bytes in cases {
            let text = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
            assert_eq!(base64url(&bytes), text);
            assert_eq!(
                VapidPublicKey::from_base64url(&text),
                Ok(VapidPublicKey::from(bytes))
            );
        }
        assert_eq!(
            VapidPublicKey::from_base64url(P256DH_BASE64URL),
            Ok(VapidPublicKey::from(P256DH))
        );
    }

    #[test]
    fn vapid_key_from_base64url_names_the_first_character_outside_the_alphabet() {
        let invalid = |index, character| {
            Err(Error::InvalidVapidKey(VapidKeyError::Character {
                index,
                character,
            }))
        };
        let replaced = |index: usize, with: &str| {
            let mut text = P256DH_BASE64URL.to_owned();
            text.replace_range(index..=index, with);
            VapidPublicKey::from_base64url(&text)
        };
        assert_eq!(replaced(10, "+"), invalid(10, '+'));
        assert_eq!(replaced(0, "/"), invalid(0, '/'));
        assert_eq!(replaced(86, " "), invalid(86, ' '));
        assert_eq!(replaced(5, "é"), invalid(5, 'é'));
        let padded = format!("{P256DH_BASE64URL}=");
        assert_eq!(VapidPublicKey::from_base64url(&padded), invalid(87, '='));
        assert_eq!(VapidPublicKey::from_base64url("AB+"), invalid(2, '+'));
    }

    #[test]
    fn vapid_key_from_base64url_rejects_any_length_but_87() {
        for length in [0, 2, 86, 88, 116] {
            assert_eq!(
                VapidPublicKey::from_base64url(&"A".repeat(length)),
                Err(Error::InvalidVapidKey(VapidKeyError::Length(length)))
            );
        }
    }

    #[test]
    fn base64url_decode_agrees_with_the_base64_crate() {
        let mut state = 1u32;
        for length in [0usize, 1, 2, 3, 4, 5, 63, 1000, 4096] {
            let bytes = (0..length)
                .map(|_| {
                    state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    state.to_be_bytes()[0]
                })
                .collect::<Vec<_>>();
            let text = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes);
            assert_eq!(base64url(&bytes), text);
            assert_eq!(base64url_decode(&text), Some(bytes));
        }
    }

    #[test]
    fn base64url_decode_rejects_characters_outside_the_alphabet() {
        for character in ['+', '/', '=', ' ', '\t', 'é'] {
            assert_eq!(base64url_decode(&format!("AB{character}")), None);
        }
    }

    #[test]
    fn base64url_decode_rejects_a_length_of_one_modulo_four() {
        for length in [1usize, 5, 9, 1001] {
            assert_eq!(base64url_decode(&"A".repeat(length)), None);
        }
    }
}
