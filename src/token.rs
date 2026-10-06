use std::time::SystemTime;

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
pub struct WebPushKeys {
    /// The subscription's P-256 public key.
    pub p256dh: String,
    /// The subscription's authentication secret.
    pub auth: String,
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

    /// The Web Push subscription keys, base64url encoded as a server expects them.
    #[must_use]
    pub fn web_push_keys(&self) -> Option<WebPushKeys> {
        let Self::WebPush { p256dh, auth, .. } = self else {
            return None;
        };
        Some(WebPushKeys {
            p256dh: base64url(p256dh),
            auth: base64url(auth),
        })
    }
}

/// RFC 4648 base64 with the URL and filename safe alphabet and no padding.
fn base64url(bytes: &[u8]) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The user agent key and authentication secret of RFC 8291, section 5.
    #[test]
    fn web_push_keys_match_rfc_8291_example() {
        let keys = web_push(P256DH, AUTH).web_push_keys().unwrap();
        assert_eq!(keys.auth, "BTBZMqHH6r4Tts7J_aSIgg");
        assert_eq!(
            keys.p256dh,
            "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4"
        );
    }

    #[test]
    fn web_push_keys_use_the_url_alphabet_without_padding() {
        let mut auth = [0u8; 16];
        auth[..3].copy_from_slice(&[0xfb, 0xff, 0xbf]);
        let mut p256dh = [0xffu8; 65];
        p256dh[0] = 0xfb;
        let keys = web_push(p256dh, auth).web_push_keys().unwrap();
        assert_eq!(keys.auth, "-_-_AAAAAAAAAAAAAAAAAA");
        assert!(keys.p256dh.starts_with("-___"));
        assert!(keys.p256dh.ends_with("__8"));
        assert_eq!(keys.p256dh.len(), 87);
    }

    #[test]
    fn web_push_keys_is_none_for_other_transports() {
        assert_eq!(Token::Apns(vec![1, 2]).web_push_keys(), None);
    }
}
