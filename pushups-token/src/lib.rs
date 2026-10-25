#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

use std::time::SystemTime;

#[cfg(feature = "serde")]
mod serde_impl;

/// The key of the FCM data map and of the APNs payload, and the one member of the WNS body, that
/// carries an encrypted push as unpadded base64url.
pub const ENCRYPTED_FIELD: &str = "pushups";

/// A push token, the address the app's server sends pushes to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(tag = "transport", rename_all = "snake_case", deny_unknown_fields)
)]
#[non_exhaustive]
pub enum Token {
    /// An APNs device token.
    Apns {
        /// The device token. Apple gives no guarantee on its length, which is 32 bytes today.
        #[cfg_attr(feature = "serde", serde(with = "serde_impl::bytes"))]
        device_token: Vec<u8>,
        /// The keys pushes to this device are encrypted for, when the app stores them.
        keys: Option<EncryptionKeys>,
    },
    /// An FCM registration token.
    Fcm {
        /// The registration token.
        token: String,
        /// The keys pushes to this device are encrypted for, when the app stores them.
        keys: Option<EncryptionKeys>,
    },
    /// A WNS channel.
    Wns {
        /// The channel URI the server posts to.
        channel_uri: String,
        /// When the channel stops accepting pushes.
        #[cfg_attr(feature = "serde", serde(with = "serde_impl::time"))]
        expires: SystemTime,
        /// The keys pushes to this device are encrypted for, when the app stores them.
        keys: Option<EncryptionKeys>,
    },
    /// A Web Push subscription.
    WebPush {
        /// The push service URL the server posts to.
        endpoint: String,
        /// The subscription's P-256 public key, uncompressed.
        #[cfg_attr(feature = "serde", serde(with = "serde_impl::fixed"))]
        p256dh: [u8; 65],
        /// The subscription's authentication secret.
        #[cfg_attr(feature = "serde", serde(with = "serde_impl::fixed"))]
        auth: [u8; 16],
        /// When the subscription expires, if the push service set an expiry.
        #[cfg_attr(feature = "serde", serde(with = "serde_impl::optional_time"))]
        expires: Option<SystemTime>,
    },
}

/// The RFC 8291 keys a device decrypts its pushes with, the public half the server encrypts for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct EncryptionKeys {
    /// The device's P-256 public key, uncompressed.
    #[cfg_attr(feature = "serde", serde(with = "serde_impl::fixed"))]
    pub p256dh: [u8; 65],
    /// The device's authentication secret.
    #[cfg_attr(feature = "serde", serde(with = "serde_impl::fixed"))]
    pub auth: [u8; 16],
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
    #[cfg_attr(feature = "serde", serde(with = "serde_impl::optional_time"))]
    pub expiration_time: Option<SystemTime>,
    /// The subscription's keys.
    pub keys: WebPushKeys,
}

impl Token {
    /// The APNs device token as lowercase hex, the form APNs request paths and `a2` take.
    #[must_use]
    pub fn apns_device_token(&self) -> Option<String> {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let Self::Apns { device_token, .. } = self else {
            return None;
        };
        let mut hex = String::with_capacity(device_token.len() * 2);
        for byte in device_token {
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
                p256dh: base64url::encode(p256dh),
                auth: base64url::encode(auth),
            },
        })
    }
}

/// RFC 4648 base64 with the URL and filename safe alphabet and no padding, for `pushups` and
/// `pushups-sender`.
#[doc(hidden)]
pub mod base64url {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    /// Encodes `bytes` without padding.
    #[must_use]
    pub fn encode(bytes: &[u8]) -> String {
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

    /// Decodes unpadded base64url, or none when `text` is not unpadded base64url.
    ///
    /// The unused bits of a trailing sextet may hold anything, so this discards them where the
    /// reference decoders reject them.
    #[must_use]
    pub fn decode(text: &str) -> Option<Vec<u8>> {
        if text.len() % 4 == 1 {
            return None;
        }
        let mut bytes = Vec::with_capacity(text.len() / 4 * 3 + 2);
        let mut pending = 0u16;
        let mut pending_bits = 0;
        for byte in text.bytes() {
            pending = (pending << 6) | u16::from(sextet(byte)?);
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

    /// The six bits `byte` encodes, or none when it is outside the alphabet.
    #[must_use]
    pub fn sextet(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use base64::Engine as _;

    use super::*;

    /// The user agent key and authentication secret of RFC 8291, section 5.
    pub(crate) const AUTH: [u8; 16] = [
        0x05, 0x30, 0x59, 0x32, 0xa1, 0xc7, 0xea, 0xbe, 0x13, 0xb6, 0xce, 0xc9, 0xfd, 0xa4, 0x88,
        0x82,
    ];
    pub(crate) const P256DH: [u8; 65] = [
        0x04, 0x25, 0x71, 0xb2, 0xbe, 0xcd, 0xfd, 0xe3, 0x60, 0x55, 0x1a, 0xaf, 0x1e, 0xd0, 0xf4,
        0xcd, 0x36, 0x6c, 0x11, 0xce, 0xbe, 0x55, 0x5f, 0x89, 0xbc, 0xb7, 0xb1, 0x86, 0xa5, 0x33,
        0x39, 0x17, 0x31, 0x68, 0xec, 0xe2, 0xeb, 0xe0, 0x18, 0x59, 0x7b, 0xd3, 0x04, 0x79, 0xb8,
        0x6e, 0x3c, 0x8f, 0x8e, 0xce, 0xd5, 0x77, 0xca, 0x59, 0x18, 0x7e, 0x92, 0x46, 0x99, 0x0d,
        0xb6, 0x82, 0x00, 0x8b, 0x0e,
    ];
    pub(crate) const P256DH_BASE64URL: &str =
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
    pub(crate) const AUTH_BASE64URL: &str = "BTBZMqHH6r4Tts7J_aSIgg";

    fn web_push(p256dh: [u8; 65], auth: [u8; 16]) -> Token {
        Token::WebPush {
            endpoint: "https://push.example.net/sub".to_owned(),
            p256dh,
            auth,
            expires: None,
        }
    }

    fn fcm(token: &str) -> Token {
        Token::Fcm {
            token: token.to_owned(),
            keys: None,
        }
    }

    #[test]
    fn apns_device_token_is_lowercase_hex_of_every_byte() {
        let token = Token::Apns {
            device_token: vec![0x00, 0x0f, 0xa0, 0xff, 0x5c],
            keys: None,
        };
        assert_eq!(token.apns_device_token().as_deref(), Some("000fa0ff5c"));
    }

    #[test]
    fn apns_device_token_is_none_for_other_transports() {
        assert_eq!(fcm("abc").apns_device_token(), None);
        assert_eq!(web_push(P256DH, AUTH).apns_device_token(), None);
    }

    fn keys(token: &Token) -> WebPushKeys {
        token.web_push_subscription().unwrap().keys
    }

    #[test]
    fn web_push_keys_match_rfc_8291_example() {
        let keys = keys(&web_push(P256DH, AUTH));
        assert_eq!(keys.auth, AUTH_BASE64URL);
        assert_eq!(keys.p256dh, P256DH_BASE64URL);
    }

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
        let apns = Token::Apns {
            device_token: vec![1, 2],
            keys: None,
        };
        assert_eq!(apns.web_push_subscription(), None);
        assert_eq!(fcm("abc").web_push_subscription(), None);
        let wns = Token::Wns {
            channel_uri: "https://wns.example.net/c".to_owned(),
            expires: SystemTime::UNIX_EPOCH,
            keys: None,
        };
        assert_eq!(wns.web_push_subscription(), None);
    }

    fn pseudo_random_bytes(length: usize, state: &mut u32) -> Vec<u8> {
        (0..length)
            .map(|_| {
                *state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                state.to_be_bytes()[0]
            })
            .collect()
    }

    #[test]
    fn base64url_agrees_with_the_base64_crate() {
        let mut state = 1u32;
        for length in [0usize, 1, 2, 3, 4, 5, 63, 65, 1000, 4096] {
            let bytes = pseudo_random_bytes(length, &mut state);
            let text = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes);
            assert_eq!(base64url::encode(&bytes), text);
            assert_eq!(base64url::decode(&text), Some(bytes));
        }
    }

    #[test]
    fn base64url_decode_rejects_characters_outside_the_alphabet() {
        for character in ['+', '/', '=', ' ', '\t', 'é'] {
            assert_eq!(base64url::decode(&format!("AB{character}")), None);
        }
    }

    #[test]
    fn base64url_decode_rejects_a_length_of_one_modulo_four() {
        for length in [1usize, 5, 9, 1001] {
            assert_eq!(base64url::decode(&"A".repeat(length)), None);
        }
    }
}
