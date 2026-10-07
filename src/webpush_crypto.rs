//! RFC 8291 Web Push message decryption, the `aes128gcm` content coding of RFC 8188, and the
//! P-256 key pair the Linux `UnifiedPush` connector keeps per subscription.
//!
//! A message body is an 86-octet header, salt, record size, `idlen` and 65-byte `keyid`,
//! followed by a single record of GCM ciphertext and its 16-octet tag.

use aes_gcm::Aes128Gcm;
use aes_gcm::aead::{Aead, KeyInit};
use hkdf::Hkdf;
use p256::elliptic_curve::point::AffineCoordinates;
use p256::{PublicKey, SecretKey};
use sha2::Sha256;

const SALT_LEN: usize = 16;
const RECORD_SIZE_LEN: usize = 4;
const KEYID_LEN: usize = 65;
const AUTH_LEN: usize = 16;
const KEY_LEN: usize = 32 + AUTH_LEN;
const RECORD_TAG_LEN: usize = 16;
const MIN_RECORD_LEN: usize = 1 + RECORD_TAG_LEN;
const MIN_RECORD_SIZE: u32 = 18;
const UNCOMPRESSED_PREFIX: u8 = 0x04;
const PADDING_DELIMITER: u8 = 0x02;
const KEY_INFO_LABEL: &[u8] = b"WebPush: info";
const KEY_INFO_LABEL_LEN: usize = 13;
const KEY_INFO_LEN: usize = KEY_INFO_LABEL_LEN + 1 + 2 * KEYID_LEN;
const CEK_INFO: [u8; 28] = *b"Content-Encoding: aes128gcm\0";
const NONCE_INFO: [u8; 24] = *b"Content-Encoding: nonce\0";

/// The user agent key pair for one Web Push subscription, a P-256 secret key and the
/// 16-byte authentication secret.
pub(crate) struct Keys {
    secret: SecretKey,
    auth: [u8; AUTH_LEN],
}

impl core::fmt::Debug for Keys {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Keys")
            .field("public_key", &self.public_key())
            .finish()
    }
}

/// A failure to decrypt a Web Push message body.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DecryptError {
    /// The randomness source failed, so no fresh key pair could be built.
    #[error("could not read random bytes")]
    Randomness,
    /// The body is shorter than the `aes128gcm` header.
    #[error("the body is shorter than the aes128gcm header")]
    HeaderTooShort,
    /// The `idlen` field is not 65, so the `keyid` is not an uncompressed point.
    #[error("the keyid is not 65 octets")]
    BadKeyIdLength,
    /// The `keyid` is not a valid uncompressed P-256 point.
    #[error("the keyid is not a valid P-256 point")]
    InvalidSenderKey,
    /// The `rs` field is below 18, or the record is shorter than a delimiter and the GCM tag.
    #[error("the record size or the record is too small")]
    BadRecordSize,
    /// The body carries more than one record, but Web Push sends exactly one.
    #[error("the body holds more than one record")]
    MultipleRecords,
    /// The GCM authentication tag does not verify.
    #[error("the record failed authentication")]
    AuthenticationFailure,
    /// The record plaintext has no non-zero octet at all.
    #[error("the record has no padding delimiter")]
    MissingPaddingDelimiter,
    /// The last non-zero octet of the record plaintext is not `0x02`.
    #[error("the padding delimiter is not 0x02")]
    BadPaddingDelimiter,
    /// HKDF-Expand rejected the output length.
    #[error("the key derivation failed")]
    KeyDerivation,
}

impl Keys {
    /// Generate a fresh P-256 secret key and authentication secret.
    pub(crate) fn generate() -> Result<Self, DecryptError> {
        let mut secret = [0u8; 32];
        let mut auth = [0u8; AUTH_LEN];
        getrandom::fill(&mut secret).map_err(|_| DecryptError::Randomness)?;
        getrandom::fill(&mut auth).map_err(|_| DecryptError::Randomness)?;
        let secret = SecretKey::from_bytes(&secret.into()).map_err(|_| DecryptError::Randomness)?;
        Ok(Self { secret, auth })
    }

    /// Read a key pair from its 48-byte serialization, the 32-byte secret scalar followed by
    /// the 16-byte authentication secret.
    pub(crate) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != KEY_LEN {
            return None;
        }
        let (secret_bytes, auth) = bytes.split_at(32);
        let secret_bytes: [u8; 32] = secret_bytes.try_into().ok()?;
        let auth: [u8; AUTH_LEN] = auth.try_into().ok()?;
        let secret = SecretKey::from_bytes(&secret_bytes.into()).ok()?;
        Some(Self { secret, auth })
    }

    /// Serialize the key pair as the 32-byte secret scalar and the 16-byte authentication
    /// secret.
    pub(crate) fn to_bytes(&self) -> [u8; KEY_LEN] {
        let mut bytes = [0u8; KEY_LEN];
        let (secret_part, auth_part) = bytes.split_at_mut(32);
        secret_part.copy_from_slice(&self.secret.to_bytes());
        auth_part.copy_from_slice(&self.auth);
        bytes
    }

    /// The uncompressed P-256 public key, the `p256dh` the subscription exposes.
    pub(crate) fn public_key(&self) -> [u8; KEYID_LEN] {
        let public = self.secret.public_key();
        let point = public.as_affine();
        let mut bytes = [0u8; KEYID_LEN];
        let (prefix, coordinates) = bytes.split_at_mut(1);
        if let Some(slot) = prefix.first_mut() {
            *slot = UNCOMPRESSED_PREFIX;
        }
        for (slot, coordinate) in coordinates
            .iter_mut()
            .zip(point.x().iter().chain(point.y().iter()))
        {
            *slot = *coordinate;
        }
        bytes
    }

    /// The 16-byte authentication secret.
    pub(crate) fn auth(&self) -> [u8; AUTH_LEN] {
        self.auth
    }

    /// Decrypt a whole `aes128gcm` body, the header and its single record.
    pub(crate) fn decrypt(&self, body: &[u8]) -> Result<Vec<u8>, DecryptError> {
        let Some((salt, rest)) = body.split_first_chunk::<SALT_LEN>() else {
            return Err(DecryptError::HeaderTooShort);
        };
        let Some((record_size_bytes, rest)) = rest.split_first_chunk::<RECORD_SIZE_LEN>() else {
            return Err(DecryptError::HeaderTooShort);
        };
        let Some((idlen, rest)) = rest.split_first() else {
            return Err(DecryptError::HeaderTooShort);
        };
        if usize::from(*idlen) != KEYID_LEN {
            return Err(DecryptError::BadKeyIdLength);
        }
        let Some((keyid, record)) = rest.split_first_chunk::<KEYID_LEN>() else {
            return Err(DecryptError::HeaderTooShort);
        };
        let record_size = u32::from_be_bytes(*record_size_bytes);
        if record_size < MIN_RECORD_SIZE {
            return Err(DecryptError::BadRecordSize);
        }
        if keyid.first() != Some(&UNCOMPRESSED_PREFIX) {
            return Err(DecryptError::InvalidSenderKey);
        }
        let sender =
            PublicKey::from_sec1_bytes(keyid).map_err(|_| DecryptError::InvalidSenderKey)?;
        if record.len() < MIN_RECORD_LEN {
            return Err(DecryptError::BadRecordSize);
        }
        if record.len() > usize::try_from(record_size).unwrap_or(usize::MAX) {
            return Err(DecryptError::MultipleRecords);
        }
        let shared = self.secret.diffie_hellman(&sender);
        let (content_key, nonce) = derive_content_key(
            &self.auth,
            salt,
            &self.public_key(),
            keyid,
            shared.raw_secret_bytes(),
        )?;
        let cipher = Aes128Gcm::new(&content_key.into());
        let plaintext = cipher
            .decrypt(&nonce.into(), record)
            .map_err(|_| DecryptError::AuthenticationFailure)?;
        let Some(delimiter) = plaintext.iter().rposition(|octet| *octet != 0) else {
            return Err(DecryptError::MissingPaddingDelimiter);
        };
        let (message, tail) = plaintext.split_at(delimiter);
        if tail.first() != Some(&PADDING_DELIMITER) {
            return Err(DecryptError::BadPaddingDelimiter);
        }
        Ok(message.to_vec())
    }
}

/// Derive the AES-GCM content key and nonce, the RFC 8291 key combining and the RFC 8188
/// content-encryption key derivation.
fn derive_content_key(
    auth: &[u8; AUTH_LEN],
    salt: &[u8],
    ua_public: &[u8; KEYID_LEN],
    sender_public: &[u8],
    ecdh: &[u8],
) -> Result<([u8; 16], [u8; 12]), DecryptError> {
    let mut key_info = [0u8; KEY_INFO_LEN];
    key_info[..KEY_INFO_LABEL_LEN].copy_from_slice(KEY_INFO_LABEL);
    key_info[KEY_INFO_LABEL_LEN + 1..KEY_INFO_LABEL_LEN + 1 + KEYID_LEN].copy_from_slice(ua_public);
    key_info[KEY_INFO_LABEL_LEN + 1 + KEYID_LEN..].copy_from_slice(sender_public);
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(auth), ecdh)
        .expand(&key_info, &mut ikm)
        .map_err(|_| DecryptError::KeyDerivation)?;
    let hkdf = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let mut content_key = [0u8; 16];
    hkdf.expand(&CEK_INFO, &mut content_key)
        .map_err(|_| DecryptError::KeyDerivation)?;
    let mut nonce = [0u8; 12];
    hkdf.expand(&NONCE_INFO, &mut nonce)
        .map_err(|_| DecryptError::KeyDerivation)?;
    Ok((content_key, nonce))
}

#[cfg(test)]
mod tests {
    use super::*;

    const UA_PRIVATE: &str = "q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94";
    const AUTH_SECRET: &str = "BTBZMqHH6r4Tts7J_aSIgg";
    const UA_PUBLIC: &str =
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
    const PLAINTEXT: &str = "When I grow up, I want to be a watermelon";
    const BODY: &str = "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN";
    const RECORD_SIZE: u32 = 4096;
    const IDLEN_LEN: usize = 1;
    const MIN_HEADER_LEN: usize = SALT_LEN + RECORD_SIZE_LEN + IDLEN_LEN;
    const HEADER_LEN: usize = MIN_HEADER_LEN + KEYID_LEN;

    fn b64url(input: &str) -> Vec<u8> {
        fn value(byte: u8) -> u32 {
            const ALPHABET: &[u8; 64] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
            ALPHABET
                .iter()
                .position(|candidate| *candidate == byte)
                .map_or_else(
                    || panic!("unexpected base64url byte {byte}"),
                    |index| u32::try_from(index).unwrap(),
                )
        }
        let mut buffer = 0u32;
        let mut bits = 0u32;
        let mut out = Vec::new();
        for byte in input.bytes() {
            buffer = (buffer << 6) | value(byte);
            bits += 6;
            if bits >= 8 {
                let take = bits - 8;
                let octet = (buffer >> take) & 0xff;
                buffer &= (1 << take) - 1;
                bits = take;
                out.push(u8::try_from(octet).unwrap());
            }
        }
        out
    }

    fn appendix_keys() -> Keys {
        let mut bytes = b64url(UA_PRIVATE);
        bytes.extend(b64url(AUTH_SECRET));
        Keys::from_bytes(&bytes).unwrap()
    }

    fn appendix_body() -> Vec<u8> {
        b64url(BODY)
    }

    fn random_secret() -> SecretKey {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).unwrap();
        SecretKey::from_bytes(&bytes.into()).unwrap()
    }

    /// Encrypt a record with the sender's ephemeral key, the same RFC 8291 and RFC 8188 steps
    /// in reverse.
    fn encrypt(ua: &Keys, sender: &SecretKey, salt: &[u8; 16], record_plain: &[u8]) -> Vec<u8> {
        let sender_bytes: [u8; KEYID_LEN] = sender
            .public_key()
            .to_sec1_bytes()
            .as_ref()
            .try_into()
            .unwrap();
        let ua_public = ua.public_key();
        let ua_public = PublicKey::from_sec1_bytes(&ua_public).unwrap();
        let shared = sender.diffie_hellman(&ua_public);
        let (content_key, nonce) = derive_content_key(
            &ua.auth(),
            salt,
            &ua.public_key(),
            &sender_bytes,
            shared.raw_secret_bytes(),
        )
        .unwrap();
        let cipher = Aes128Gcm::new(&content_key.into());
        let record = cipher.encrypt(&nonce.into(), record_plain).unwrap();
        let mut body = Vec::with_capacity(HEADER_LEN + record.len());
        body.extend_from_slice(salt);
        body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
        body.push(u8::try_from(KEYID_LEN).unwrap());
        body.extend_from_slice(&sender_bytes);
        body.extend_from_slice(&record);
        body
    }

    #[test]
    fn appendix_a_body_decrypts() {
        let keys = appendix_keys();
        let public_key: [u8; KEYID_LEN] = b64url(UA_PUBLIC).try_into().unwrap();
        let auth: [u8; AUTH_LEN] = b64url(AUTH_SECRET).try_into().unwrap();
        assert_eq!(keys.public_key(), public_key);
        assert_eq!(keys.auth(), auth);
        assert_eq!(
            keys.decrypt(&appendix_body()).unwrap(),
            PLAINTEXT.as_bytes()
        );
    }

    #[test]
    fn key_bytes_round_trip() {
        let keys = appendix_keys();
        let restored = Keys::from_bytes(&keys.to_bytes()).unwrap();
        assert_eq!(restored.to_bytes(), keys.to_bytes());
        assert_eq!(restored.public_key(), keys.public_key());
        assert_eq!(restored.auth(), keys.auth());
        let body = appendix_body();
        assert_eq!(
            restored.decrypt(&body).unwrap(),
            keys.decrypt(&body).unwrap()
        );
    }

    #[test]
    fn generated_keys_decrypt_rfc8291_body() {
        let ua = Keys::generate().unwrap();
        let sender = random_secret();
        let mut salt = [0u8; 16];
        getrandom::fill(&mut salt).unwrap();
        let mut record_plain = PLAINTEXT.as_bytes().to_vec();
        record_plain.push(PADDING_DELIMITER);
        record_plain.extend_from_slice(&[0u8; 8]);
        let body = encrypt(&ua, &sender, &salt, &record_plain);
        assert_eq!(ua.decrypt(&body).unwrap(), PLAINTEXT.as_bytes());
    }

    #[test]
    fn wrong_auth_fails_authentication() {
        let mut auth = b64url(AUTH_SECRET);
        *auth.get_mut(0).unwrap() ^= 0xff;
        let mut bytes = b64url(UA_PRIVATE);
        bytes.extend(auth);
        let keys = Keys::from_bytes(&bytes).unwrap();
        assert_eq!(
            keys.decrypt(&appendix_body()),
            Err(DecryptError::AuthenticationFailure)
        );
    }

    #[test]
    fn truncated_header_fails() {
        let keys = appendix_keys();
        let body = appendix_body();
        assert_eq!(keys.decrypt(&body[..10]), Err(DecryptError::HeaderTooShort));
        assert_eq!(
            keys.decrypt(&body[..HEADER_LEN - 1]),
            Err(DecryptError::HeaderTooShort)
        );
    }

    #[test]
    fn flipped_ciphertext_fails() {
        let keys = appendix_keys();
        let mut body = appendix_body();
        *body.get_mut(HEADER_LEN + 4).unwrap() ^= 1;
        assert_eq!(
            keys.decrypt(&body),
            Err(DecryptError::AuthenticationFailure)
        );
    }

    #[test]
    fn wrong_key_id_length_fails() {
        let keys = appendix_keys();
        let mut body = appendix_body();
        *body.get_mut(MIN_HEADER_LEN - 1).unwrap() = 64;
        assert_eq!(keys.decrypt(&body), Err(DecryptError::BadKeyIdLength));
    }

    #[test]
    fn invalid_sender_key_fails() {
        let keys = appendix_keys();
        let mut body = appendix_body();
        let mut point = [0u8; KEYID_LEN];
        let (prefix, coordinates) = point.split_first_mut().unwrap();
        *prefix = UNCOMPRESSED_PREFIX;
        *coordinates.get_mut(31).unwrap() = 1;
        *coordinates.get_mut(63).unwrap() = 2;
        body.get_mut(MIN_HEADER_LEN..HEADER_LEN)
            .unwrap()
            .copy_from_slice(&point);
        assert_eq!(keys.decrypt(&body), Err(DecryptError::InvalidSenderKey));
    }

    #[test]
    fn bad_record_size_fails() {
        let keys = appendix_keys();
        let mut small_rs = appendix_body();
        small_rs
            .get_mut(SALT_LEN..SALT_LEN + RECORD_SIZE_LEN)
            .unwrap()
            .copy_from_slice(&(MIN_RECORD_SIZE - 1).to_be_bytes());
        assert_eq!(keys.decrypt(&small_rs), Err(DecryptError::BadRecordSize));
        let body = appendix_body();
        assert_eq!(
            keys.decrypt(&body[..HEADER_LEN]),
            Err(DecryptError::BadRecordSize)
        );
        assert_eq!(
            keys.decrypt(&body[..HEADER_LEN + MIN_RECORD_LEN - 1]),
            Err(DecryptError::BadRecordSize)
        );
    }

    #[test]
    fn multiple_records_fail() {
        let keys = appendix_keys();
        let mut body = appendix_body();
        body.get_mut(SALT_LEN..SALT_LEN + RECORD_SIZE_LEN)
            .unwrap()
            .copy_from_slice(&50u32.to_be_bytes());
        assert_eq!(keys.decrypt(&body), Err(DecryptError::MultipleRecords));
    }

    #[test]
    fn missing_padding_delimiter_fails() {
        let ua = Keys::generate().unwrap();
        let sender = random_secret();
        let mut salt = [0u8; 16];
        getrandom::fill(&mut salt).unwrap();
        let body = encrypt(&ua, &sender, &salt, &[0u8; 20]);
        assert_eq!(
            ua.decrypt(&body),
            Err(DecryptError::MissingPaddingDelimiter)
        );
    }

    #[test]
    fn wrong_padding_delimiter_fails() {
        let ua = Keys::generate().unwrap();
        let sender = random_secret();
        let mut salt = [0u8; 16];
        getrandom::fill(&mut salt).unwrap();
        let record_plain = [b'h', b'i', 0x01, 0, 0];
        let body = encrypt(&ua, &sender, &salt, &record_plain);
        assert_eq!(ua.decrypt(&body), Err(DecryptError::BadPaddingDelimiter));
    }
}
