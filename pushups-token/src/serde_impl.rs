//! The JSON shapes of the token's fields: bytes as unpadded base64url, times as milliseconds
//! since the Unix epoch.

use std::time::{Duration, SystemTime};

use serde::de::Error as _;
use serde::{Deserialize as _, Deserializer, Serializer};

use crate::base64url;

fn decode<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    let text = std::borrow::Cow::<str>::deserialize(deserializer)?;
    base64url::decode(&text).ok_or_else(|| D::Error::custom("not unpadded base64url"))
}

fn milliseconds<E: serde::ser::Error>(time: SystemTime) -> Result<u64, E> {
    let since_epoch = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| E::custom("the time is before the Unix epoch"))?;
    u64::try_from(since_epoch.as_millis())
        .map_err(|_| E::custom("the time overflows u64 milliseconds"))
}

fn from_milliseconds<E: serde::de::Error>(milliseconds: u64) -> Result<SystemTime, E> {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_millis(milliseconds))
        .ok_or_else(|| E::custom("the time overflows SystemTime"))
}

pub(crate) mod bytes {
    use super::{Deserializer, Serializer, base64url};

    pub(crate) fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64url::encode(bytes))
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<u8>, D::Error> {
        super::decode(deserializer)
    }
}

pub(crate) mod fixed {
    use super::{Deserializer, Serializer, base64url};

    pub(crate) fn serialize<S: Serializer, const N: usize>(
        bytes: &[u8; N],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64url::encode(bytes))
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
        deserializer: D,
    ) -> Result<[u8; N], D::Error> {
        let bytes = super::decode(deserializer)?;
        let length = bytes.len();
        bytes.try_into().map_err(|_| {
            serde::de::Error::custom(format_args!("{length} bytes where {N} are expected"))
        })
    }
}

pub(crate) mod time {
    use super::{Deserializer, Serializer, SystemTime};

    #[cfg_attr(
        target_os = "windows",
        expect(
            clippy::trivially_copy_pass_by_ref,
            reason = "`with` passes the field by reference, and Windows's `SystemTime` is 8 bytes"
        )
    )]
    pub(crate) fn serialize<S: Serializer>(
        time: &SystemTime,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(super::milliseconds(*time)?)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<SystemTime, D::Error> {
        super::from_milliseconds(serde::Deserialize::deserialize(deserializer)?)
    }
}

pub(crate) mod optional_time {
    use super::{Deserializer, Serializer, SystemTime};

    #[expect(
        clippy::ref_option,
        reason = "`serialize_with` passes the field by reference"
    )]
    pub(crate) fn serialize<S: Serializer>(
        time: &Option<SystemTime>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let Some(time) = time else {
            return serializer.serialize_none();
        };
        serializer.serialize_some(&super::milliseconds::<S::Error>(*time)?)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<SystemTime>, D::Error> {
        let milliseconds: Option<u64> = serde::Deserialize::deserialize(deserializer)?;
        milliseconds.map(super::from_milliseconds).transpose()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use serde_json::json;

    use crate::tests::{AUTH, AUTH_BASE64URL, P256DH, P256DH_BASE64URL};
    use crate::{EncryptionKeys, Token};

    const KEYS: EncryptionKeys = EncryptionKeys {
        p256dh: P256DH,
        auth: AUTH,
    };

    fn at(milliseconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_millis(milliseconds)
    }

    /// Every transport, with and without keys, in the JSON a server receives, and back.
    #[test]
    fn every_token_round_trips_through_its_json() {
        let keys = json!({ "p256dh": P256DH_BASE64URL, "auth": AUTH_BASE64URL });
        let cases = [
            (
                Token::Apns {
                    device_token: vec![0xfb, 0xff, 0x00],
                    keys: Some(KEYS),
                },
                json!({ "transport": "apns", "device_token": "-_8A", "keys": keys }),
            ),
            (
                Token::Fcm {
                    token: "fcm:token".to_owned(),
                    keys: None,
                },
                json!({ "transport": "fcm", "token": "fcm:token", "keys": null }),
            ),
            (
                Token::Wns {
                    channel_uri: "https://wns.example.net/c".to_owned(),
                    expires: at(1_791_881_200_386),
                    keys: Some(KEYS),
                },
                json!({
                    "transport": "wns",
                    "channel_uri": "https://wns.example.net/c",
                    "expires": 1_791_881_200_386_u64,
                    "keys": keys,
                }),
            ),
            (
                Token::WebPush {
                    endpoint: "https://push.example.net/sub".to_owned(),
                    p256dh: P256DH,
                    auth: AUTH,
                    expires: None,
                },
                json!({
                    "transport": "web_push",
                    "endpoint": "https://push.example.net/sub",
                    "p256dh": P256DH_BASE64URL,
                    "auth": AUTH_BASE64URL,
                    "expires": null,
                }),
            ),
        ];
        for (token, expected) in cases {
            assert_eq!(serde_json::to_value(&token).unwrap(), expected);
            assert_eq!(serde_json::from_value::<Token>(expected).unwrap(), token);
        }
    }

    #[test]
    fn a_key_of_the_wrong_length_is_refused() {
        let short = &P256DH_BASE64URL[..86];
        let error = serde_json::from_value::<Token>(json!({
            "transport": "fcm",
            "token": "t",
            "keys": { "p256dh": short, "auth": AUTH_BASE64URL },
        }))
        .unwrap_err();
        assert!(
            error.to_string().contains("64 bytes where 65 are expected"),
            "{error}"
        );
    }

    #[test]
    fn bytes_outside_unpadded_base64url_are_refused() {
        for device_token in ["AB+/", "AA==", "A"] {
            let error = serde_json::from_value::<Token>(json!({
                "transport": "apns",
                "device_token": device_token,
                "keys": null,
            }))
            .unwrap_err();
            assert!(
                error.to_string().contains("not unpadded base64url"),
                "{error}"
            );
        }
    }

    #[test]
    fn an_unknown_transport_or_field_is_refused() {
        assert!(
            serde_json::from_value::<Token>(json!({ "transport": "mqtt", "token": "t" })).is_err()
        );
        assert!(
            serde_json::from_value::<Token>(
                json!({ "transport": "fcm", "token": "t", "keys": null, "extra": 1 })
            )
            .is_err()
        );
    }

    #[test]
    fn a_time_before_the_epoch_does_not_serialise() {
        let token = Token::Wns {
            channel_uri: "c".to_owned(),
            expires: SystemTime::UNIX_EPOCH - Duration::from_millis(1),
            keys: None,
        };
        let error = serde_json::to_value(&token).unwrap_err();
        assert!(
            error.to_string().contains("before the Unix epoch"),
            "{error}"
        );
    }

    /// The shape a browser's `PushSubscription.toJSON()` gives, which Web Push servers read.
    #[test]
    fn web_push_subscription_serialises_as_push_subscription_to_json() {
        let token = Token::WebPush {
            endpoint: "https://push.example.net/sub?a=\"b\"".to_owned(),
            p256dh: P256DH,
            auth: AUTH,
            expires: Some(at(1_791_881_200_386)),
        };
        let expected = json!({
            "endpoint": "https://push.example.net/sub?a=\"b\"",
            "expirationTime": 1_791_881_200_386_u64,
            "keys": { "p256dh": P256DH_BASE64URL, "auth": AUTH_BASE64URL },
        });
        let json = serde_json::to_value(token.web_push_subscription().unwrap()).unwrap();
        assert_eq!(json, expected);
    }
}
