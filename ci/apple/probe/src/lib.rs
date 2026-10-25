//! The consumer transform linked into the native completion harness and iOS extension.

use std::collections::HashMap;
use std::ffi::{c_char, CString};

use keyring_core::api::CredentialStoreApi;
use p256::elliptic_curve::point::AffineCoordinates;
use pushups::{Error, Message, Notification, Rendering, RenderingContext};

#[pushups::rendering_handler]
fn render(_context: RenderingContext, message: Message) -> Result<Rendering, Error> {
    let payload: serde_json::Value = serde_json::from_slice(&message.payload)
        .map_err(|error| Error::Platform(error.to_string()))?;
    match payload["kind"].as_str().unwrap_or_default() {
        "error" => Err(Error::Platform("an error".to_owned())),
        "panic" => panic!("a panic"),
        "no-alert" => Ok(Rendering::NoAlert),
        "icon" => Ok(Notification::new("Icon title")
            .icon("https://example.com/icon.png")
            .into()),
        "clear" => Ok(Notification::new("").tag("").data("").navigate("").into()),
        "minimal" => Ok(Notification::new("Minimal").into()),
        _ => {
            if let Some(sleep) = payload["sleep"].as_u64() {
                std::thread::sleep(std::time::Duration::from_millis(sleep));
            }
            Ok(Notification::new("Rendered title")
                .body("Decrypted body")
                .tag("thread-1")
                .data(payload["original"].as_str().unwrap_or("tap-data"))
                .navigate("https://example.com")
                .into())
        }
    }
}

/// Installs the crate in the process that links this probe, before the probe rotates its keys.
#[unsafe(no_mangle)]
pub extern "C" fn __pushups_probe_install() {
    let _ = pushups::install(pushups::Config::new());
}

/// Rotates the stored keys and registers again, returning zero when the rotation and the
/// registration attempt both succeed.
#[unsafe(no_mangle)]
pub extern "C" fn __pushups_probe_rotate() -> i32 {
    match pushups::rotate_keys() {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

/// Seals `plaintext` as a sender would seal it, with the key the protected keychain holds for
/// `access_group`.
///
/// # Safety
///
/// `plaintext` must point to `len` readable bytes, and `access_group` to a NUL-terminated
/// string, both live for the call. The returned pointer, when non-null, must be released
/// with `__pushups_probe_free`, exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __pushups_probe_seal(
    plaintext: *const u8,
    len: usize,
    access_group: *const c_char,
) -> *mut c_char {
    // SAFETY: the caller passed a live field of `len` bytes and a live, NUL-terminated group.
    let plaintext = unsafe { std::slice::from_raw_parts(plaintext, len) }.to_vec();
    let group = unsafe { std::ffi::CStr::from_ptr(access_group) }
        .to_str()
        .ok()
        .map(str::to_owned);
    seal(plaintext, group.as_deref())
}

/// Releases a sealed field from `__pushups_probe_seal`, exactly once.
///
/// # Safety
///
/// `sealed` must be null or a pointer `__pushups_probe_seal` returned, released once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __pushups_probe_free(sealed: *mut c_char) {
    // SAFETY: the caller passed its own pointer or null, and releases it once.
    unsafe {
        if !sealed.is_null() {
            drop(CString::from_raw(sealed));
        }
    }
}

/// The sealed field for `plaintext`, the null pointer when it cannot be sealed.
fn seal(plaintext: Vec<u8>, access_group: Option<&str>) -> *mut c_char {
    let Ok(field) = sealed_field(&plaintext, access_group) else {
        return std::ptr::null_mut();
    };
    CString::new(field)
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

/// A failure to seal the probe's plaintext.
#[derive(Debug, thiserror::Error)]
enum SealError {
    /// The keyring store failed.
    #[error("the keyring store failed")]
    Keyring(#[source] keyring_core::Error),
    /// The store's key is not the 48-byte key pair.
    #[error("the store's key is not 48 bytes")]
    KeyLength,
    /// The stored key is not a valid P-256 scalar.
    #[error("the stored key is not a valid P-256 scalar")]
    Scalar(#[source] p256::elliptic_curve::Error),
    /// The sealing failed.
    #[error("sealing failed")]
    Seal(#[source] pushups_sender::SealError),
}

/// The base64url field a sender would seal for `plaintext`, with the key the store holds for
/// `access_group`.
fn sealed_field(plaintext: &[u8], access_group: Option<&str>) -> Result<String, SealError> {
    let mut configuration = HashMap::new();
    if let Some(group) = access_group {
        configuration.insert("access-group", group);
    }
    let store =
        apple_native_keyring_store::protected::Store::new_with_configuration(&configuration)
            .map_err(SealError::Keyring)?;
    let entry = store
        .build("rs.pushups", "keys", None)
        .map_err(SealError::Keyring)?;
    let secret = entry.get_secret().map_err(SealError::Keyring)?;
    let secret: [u8; 48] = secret.try_into().map_err(|_| SealError::KeyLength)?;
    let (seed, auth) = secret.split_at(32);
    let seed: [u8; 32] = seed.try_into().unwrap();
    let auth: [u8; 16] = auth.try_into().unwrap();
    let secret_key = p256::SecretKey::from_bytes(&seed.into()).map_err(SealError::Scalar)?;
    let public = secret_key.public_key();
    let point = public.as_affine();
    let mut p256dh = [0u8; 65];
    p256dh[0] = 0x04;
    p256dh[1..33].copy_from_slice(point.x().as_slice());
    p256dh[33..].copy_from_slice(point.y().as_slice());
    let token = pushups::Token::Apns {
        device_token: vec![0; 32],
        keys: Some(pushups::EncryptionKeys { p256dh, auth }),
    };
    Ok(pushups_sender::encrypt(&token, plaintext)
        .map_err(SealError::Seal)?
        .to_base64url())
}
