#![doc = include_str!("../README.md")]

use aes_gcm::Aes128Gcm;
use aes_gcm::aead::{Aead, KeyInit};
use hkdf::Hkdf;
use p256::{PublicKey, SecretKey};
use pushups_token::base64url;
use sha2::Sha256;

pub use pushups_token::{ENCRYPTED_FIELD, EncryptionKeys, Token};

const SALT_LEN: usize = 16;
const RECORD_SIZE_LEN: usize = 4;
const IDLEN_LEN: usize = 1;
const KEYID_LEN: usize = 65;
const AUTH_LEN: usize = 16;
const RECORD_TAG_LEN: usize = 16;
const HEADER_LEN: usize = SALT_LEN + RECORD_SIZE_LEN + IDLEN_LEN + KEYID_LEN;
const RECORD_SIZE: u32 = 4096;
/// The largest `plaintext` one `aes128gcm` record holds, the record minus the padding
/// delimiter and the GCM tag.
const MAX_PLAINTEXT: usize = 4096 - 1 - 16;
const PADDING_DELIMITER: u8 = 0x02;
const KEY_INFO_LABEL: &[u8] = b"WebPush: info";
const KEY_INFO_LABEL_LEN: usize = 13;
const KEY_INFO_LEN: usize = KEY_INFO_LABEL_LEN + 1 + 2 * KEYID_LEN;
const CEK_INFO: [u8; 28] = *b"Content-Encoding: aes128gcm\0";
const NONCE_INFO: [u8; 24] = *b"Content-Encoding: nonce\0";

/// A push sealed for one token, the RFC 8291 `aes128gcm` body.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct Sealed(Vec<u8>);

impl Sealed {
    /// The raw body, what a Web Push request carries with `Content-Encoding: aes128gcm`.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The raw body, owned.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// The body as unpadded base64url, the value of the FCM data key and the APNs payload key
    /// [`ENCRYPTED_FIELD`].
    #[must_use]
    pub fn to_base64url(&self) -> String {
        base64url::encode(&self.0)
    }

    /// The WNS raw body, exactly `{"pushups":"<base64url>"}` with no whitespace.
    #[must_use]
    pub fn wns_body(&self) -> String {
        format!(
            "{{\"{ENCRYPTED_FIELD}\":\"{}\"}}",
            base64url::encode(&self.0)
        )
    }
}

/// A failure to seal a push for a token.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SealError {
    /// The token carries no encryption keys.
    #[error("the token carries no encryption keys")]
    NoKeys,
    /// The token's `p256dh` key is not a valid uncompressed P-256 point.
    #[error("the p256dh key is not a valid P-256 point")]
    InvalidKey,
    /// The plaintext is longer than one `aes128gcm` record holds.
    #[error("the plaintext is {len} bytes and one record holds at most {max} bytes")]
    TooLarge {
        /// The plaintext's length in bytes.
        len: usize,
        /// The largest plaintext length one record holds, in bytes.
        max: usize,
    },
    /// The token's transport is not one `pushups-sender` can encrypt for.
    #[error("the token's transport is unsupported")]
    UnsupportedTransport,
    /// The randomness source failed, so no fresh salt or key pair could be drawn.
    #[error("the randomness source failed")]
    Randomness,
}

/// Encrypts `plaintext` for the keys `token` carries.
///
/// A fresh 16-byte salt and a fresh ephemeral P-256 key pair are drawn for every call, so two
/// seals of the same plaintext differ.
///
/// # Errors
///
/// [`SealError::NoKeys`] when the token carries no keys, [`SealError::InvalidKey`] when its
/// `p256dh` key is not a point on P-256, [`SealError::TooLarge`] when `plaintext` is longer
/// than one `aes128gcm` record holds, [`SealError::UnsupportedTransport`] for a transport this
/// crate does not cover, and [`SealError::Randomness`] when the operating system's randomness
/// source is unavailable.
pub fn encrypt(token: &Token, plaintext: &[u8]) -> Result<Sealed, SealError> {
    let keys = keys_of(token)?;
    if plaintext.len() > MAX_PLAINTEXT {
        return Err(SealError::TooLarge {
            len: plaintext.len(),
            max: MAX_PLAINTEXT,
        });
    }
    let (salt, ephemeral) = fresh_material()?;
    encrypt_with(&keys, plaintext, &ephemeral, salt)
}

/// The keys the token carries, or the error that fits.
fn keys_of(token: &Token) -> Result<EncryptionKeys, SealError> {
    match token {
        Token::Apns { keys, .. } | Token::Fcm { keys, .. } | Token::Wns { keys, .. } => {
            keys.ok_or(SealError::NoKeys)
        }
        Token::WebPush { p256dh, auth, .. } => Ok(EncryptionKeys {
            p256dh: *p256dh,
            auth: *auth,
        }),
        _ => Err(SealError::UnsupportedTransport),
    }
}

/// A fresh 16-byte salt and a fresh ephemeral P-256 key, one per `encrypt` call. The secret is
/// drawn until the scalar is below the group order, rejection sampling.
fn fresh_material() -> Result<([u8; SALT_LEN], SecretKey), SealError> {
    let mut salt = [0u8; SALT_LEN];
    getrandom::fill(&mut salt).map_err(|_| SealError::Randomness)?;
    let ephemeral = loop {
        let mut secret = [0u8; 32];
        getrandom::fill(&mut secret).map_err(|_| SealError::Randomness)?;
        if let Ok(key) = SecretKey::from_bytes(&secret.into()) {
            break key;
        }
    };
    Ok((salt, ephemeral))
}

/// The RFC 8291 steps for one record, with the caller's salt and ephemeral key.
fn encrypt_with(
    keys: &EncryptionKeys,
    plaintext: &[u8],
    ephemeral: &SecretKey,
    salt: [u8; SALT_LEN],
) -> Result<Sealed, SealError> {
    let ua_public = PublicKey::from_sec1_bytes(&keys.p256dh).map_err(|_| SealError::InvalidKey)?;
    let sender_public: [u8; KEYID_LEN] = ephemeral
        .public_key()
        .to_sec1_bytes()
        .as_ref()
        .try_into()
        .expect("an uncompressed P-256 point is 65 bytes");
    let shared = ephemeral.diffie_hellman(&ua_public);
    let (content_key, nonce) = derive_content_key(
        &keys.auth,
        &salt,
        &keys.p256dh,
        &sender_public,
        shared.raw_secret_bytes(),
    );
    let mut record_plain = Vec::with_capacity(plaintext.len() + 1);
    record_plain.extend_from_slice(plaintext);
    record_plain.push(PADDING_DELIMITER);
    let cipher = Aes128Gcm::new(&content_key.into());
    // The record is the record plaintext plus its GCM tag.
    let mut body = Vec::with_capacity(HEADER_LEN + record_plain.len() + RECORD_TAG_LEN);
    let record = cipher
        .encrypt(&nonce.into(), record_plain.as_slice())
        .expect("aes-gcm never fails to encrypt");
    body.extend_from_slice(&salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(u8::try_from(KEYID_LEN).expect("65 fits in a byte"));
    body.extend_from_slice(&sender_public);
    body.extend_from_slice(&record);
    Ok(Sealed(body))
}

/// Derive the AES-GCM content key and nonce, the RFC 8291 key combining and the RFC 8188
/// content-encryption key derivation.
fn derive_content_key(
    auth: &[u8; AUTH_LEN],
    salt: &[u8],
    ua_public: &[u8; KEYID_LEN],
    sender_public: &[u8; KEYID_LEN],
    ecdh: &[u8],
) -> ([u8; 16], [u8; 12]) {
    let mut key_info = [0u8; KEY_INFO_LEN];
    key_info[..KEY_INFO_LABEL_LEN].copy_from_slice(KEY_INFO_LABEL);
    key_info[KEY_INFO_LABEL_LEN + 1..KEY_INFO_LABEL_LEN + 1 + KEYID_LEN].copy_from_slice(ua_public);
    key_info[KEY_INFO_LABEL_LEN + 1 + KEYID_LEN..].copy_from_slice(sender_public);
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(auth), ecdh)
        .expand(&key_info, &mut ikm)
        .expect("the output fits one HKDF block");
    let hkdf = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let mut content_key = [0u8; 16];
    hkdf.expand(&CEK_INFO, &mut content_key)
        .expect("the output fits one HKDF block");
    let mut nonce = [0u8; 12];
    hkdf.expand(&NONCE_INFO, &mut nonce)
        .expect("the output fits one HKDF block");
    (content_key, nonce)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    /// The RFC 8291, section 5, receiver public key, authentication secret, sender private key
    /// and body.
    const UA_PUBLIC: &str =
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
    const AUTH_SECRET: &str = "BTBZMqHH6r4Tts7J_aSIgg";
    const SENDER_PRIVATE: &str = "yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw";
    const PLAINTEXT: &str = "When I grow up, I want to be a watermelon";
    const BODY: &str = "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN";
    const UNCOMPRESSED_PREFIX: u8 = 0x04;

    fn decode(text: &str) -> Vec<u8> {
        base64url::decode(text).expect("an RFC 8291 base64url value")
    }

    fn rfc_keys() -> EncryptionKeys {
        EncryptionKeys {
            p256dh: decode(UA_PUBLIC).try_into().expect("65 bytes"),
            auth: decode(AUTH_SECRET).try_into().expect("16 bytes"),
        }
    }

    fn fcm(keys: Option<EncryptionKeys>) -> Token {
        Token::Fcm {
            token: "fcm-registration-token".to_owned(),
            keys,
        }
    }

    #[test]
    fn the_rfc8291_vector_reproduces_the_rfc_body() {
        let body = decode(BODY);
        let salt: [u8; SALT_LEN] = body[..SALT_LEN].try_into().expect("a 16-byte salt");
        let secret: [u8; 32] = decode(SENDER_PRIVATE).try_into().expect("a 32-byte scalar");
        let ephemeral = SecretKey::from_bytes(&secret.into()).expect("the RFC 8291 sender key");
        let sealed = encrypt_with(&rfc_keys(), PLAINTEXT.as_bytes(), &ephemeral, salt)
            .expect("the RFC 8291 keys are valid");
        assert_eq!(sealed.as_bytes(), body.as_slice());
    }

    #[test]
    fn a_seal_carries_the_header_and_fresh_material_per_call() {
        let token = fcm(Some(rfc_keys()));
        let first = encrypt(&token, PLAINTEXT.as_bytes()).expect("the RFC 8291 keys are valid");
        let second = encrypt(&token, PLAINTEXT.as_bytes()).expect("the RFC 8291 keys are valid");
        let body = first.as_bytes();
        assert_eq!(body.len() - (PLAINTEXT.len() + 1 + RECORD_TAG_LEN), 86);
        assert_eq!(
            body.len(),
            HEADER_LEN + PLAINTEXT.len() + 1 + RECORD_TAG_LEN
        );
        assert_eq!(
            &body[SALT_LEN..SALT_LEN + RECORD_SIZE_LEN],
            RECORD_SIZE.to_be_bytes()
        );
        assert_eq!(
            body[SALT_LEN + RECORD_SIZE_LEN],
            u8::try_from(KEYID_LEN).expect("65 fits in a byte")
        );
        assert_eq!(body[SALT_LEN + RECORD_SIZE_LEN + 1], UNCOMPRESSED_PREFIX);
        assert!(
            PublicKey::from_sec1_bytes(&body[SALT_LEN + RECORD_SIZE_LEN + 1..HEADER_LEN]).is_ok()
        );
        assert_ne!(first.as_bytes(), second.as_bytes());
    }

    #[test]
    fn a_keyed_transport_without_keys_is_a_no_keys_error() {
        let tokens = [
            Token::Apns {
                device_token: vec![1],
                keys: None,
            },
            fcm(None),
            Token::Wns {
                channel_uri: "https://channel.windows.net/".to_owned(),
                expires: SystemTime::now(),
                keys: None,
            },
        ];
        for token in &tokens {
            assert_eq!(encrypt(token, b"hello"), Err(SealError::NoKeys));
        }
    }

    #[test]
    fn a_p256dh_off_the_curve_is_an_invalid_key() {
        let mut p256dh = [0u8; KEYID_LEN];
        let (prefix, coordinates) = p256dh.split_first_mut().expect("65 bytes");
        *prefix = UNCOMPRESSED_PREFIX;
        *coordinates.get_mut(31).expect("an x coordinate slot") = 1;
        *coordinates.get_mut(63).expect("a y coordinate slot") = 2;
        let token = fcm(Some(EncryptionKeys {
            p256dh,
            auth: [7; AUTH_LEN],
        }));
        assert_eq!(encrypt(&token, b"hello"), Err(SealError::InvalidKey));
    }

    #[test]
    fn a_plaintext_beyond_one_record_is_too_large() {
        let token = fcm(Some(rfc_keys()));
        assert_eq!(
            encrypt(&token, &[0u8; MAX_PLAINTEXT + 1]),
            Err(SealError::TooLarge {
                len: MAX_PLAINTEXT + 1,
                max: MAX_PLAINTEXT,
            })
        );
        assert!(encrypt(&token, &[0u8; MAX_PLAINTEXT]).is_ok());
    }

    #[test]
    fn a_wns_body_is_the_envelope_of_its_base64url_body() {
        let sealed = Sealed(decode(BODY));
        assert_eq!(sealed.to_base64url(), BODY);
        assert_eq!(
            sealed.wns_body(),
            format!("{{\"{ENCRYPTED_FIELD}\":\"{BODY}\"}}")
        );
    }
}
