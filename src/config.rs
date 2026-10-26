use crate::{Error, VapidKeyError};
use pushups_token::base64url;

/// What [`install`](crate::install) needs beyond the platform's own configuration.
///
/// Each push service that needs values from the app gets its own part, set with its own
/// method, so the same setup code compiles on every target. Apple and Android's FCM need none,
/// since Android reads Firebase's values through [`firebase_config!`](crate::firebase_config).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub(crate) web: Option<WebPushConfig>,
    pub(crate) unified_push: Option<UnifiedPushConfig>,
    pub(crate) linux: Option<LinuxConfig>,
    pub(crate) windows: Option<WnsConfig>,
}

impl Config {
    /// A configuration with no service-specific part.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the Web Push part, which the web target requires.
    #[must_use]
    pub fn web(mut self, web: WebPushConfig) -> Self {
        self.web = Some(web);
        self
    }

    /// Sets the UnifiedPush part, used on Linux and by UnifiedPush on Android.
    #[must_use]
    pub fn unified_push(mut self, unified_push: UnifiedPushConfig) -> Self {
        self.unified_push = Some(unified_push);
        self
    }

    /// Sets the Linux part, which the Linux target requires.
    #[must_use]
    pub fn linux(mut self, linux: LinuxConfig) -> Self {
        self.linux = Some(linux);
        self
    }

    /// Sets the WNS part, which the Windows target requires.
    #[must_use]
    pub fn windows(mut self, wns: WnsConfig) -> Self {
        self.windows = Some(wns);
        self
    }
}

/// The VAPID public key of the app's server, an uncompressed P-256 point.
///
/// Servers publish it as base64url, which [`from_base64url`](Self::from_base64url) decodes.
/// Whether the bytes are a point on the curve is checked at registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VapidPublicKey([u8; 65]);

impl VapidPublicKey {
    /// The key from the unpadded base64url its server publishes, 87 characters.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidVapidKey`] when `text` holds a character outside the base64url alphabet,
    /// padding included, or is not 87 characters long.
    pub fn from_base64url(text: &str) -> Result<Self, Error> {
        base64url_decode_vapid_key(text)
            .map(Self)
            .map_err(Error::InvalidVapidKey)
    }

    /// The key's 65 bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 65] {
        &self.0
    }

    /// The key's 87 base64url characters, the form the UnifiedPush transports send.
    #[cfg(any(target_os = "android", target_os = "linux"))]
    pub(crate) fn as_base64url(&self) -> String {
        base64url::encode(&self.0)
    }
}

/// Decodes the 87 characters of unpadded base64url that encode a 65-byte VAPID public key.
fn base64url_decode_vapid_key(text: &str) -> Result<[u8; 65], VapidKeyError> {
    let mut bytes = [0u8; 65];
    let mut written = 0;
    let mut pending = 0u16;
    let mut pending_bits = 0;
    for (index, character) in text.char_indices() {
        let byte =
            u8::try_from(character).map_err(|_| VapidKeyError::Character { index, character })?;
        let sextet =
            base64url::sextet(byte).ok_or(VapidKeyError::Character { index, character })?;
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

impl From<[u8; 65]> for VapidPublicKey {
    fn from(bytes: [u8; 65]) -> Self {
        Self(bytes)
    }
}

/// The UnifiedPush part of [`Config`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedPushConfig {
    pub(crate) vapid_public_key: VapidPublicKey,
}

impl UnifiedPushConfig {
    /// A configuration for the VAPID public key of the app's server.
    ///
    /// Distributors that deliver through FCM refuse a registration without it.
    #[must_use]
    pub fn new(vapid_public_key: impl Into<VapidPublicKey>) -> Self {
        Self {
            vapid_public_key: vapid_public_key.into(),
        }
    }
}

/// The Linux part of [`Config`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxConfig {
    pub(crate) app_id: String,
    pub(crate) notifications: Option<AppNotifications>,
}

impl LinuxConfig {
    /// A configuration for the app's reverse-DNS id, such as `org.example.App`.
    ///
    /// The id is the app's name on the D-Bus session bus, so a distributor can deliver to it and
    /// start it for a push.
    #[must_use]
    pub fn new(app_id: impl Into<String>) -> Self {
        Self {
            app_id: app_id.into(),
            notifications: None,
        }
    }

    /// The display name and icon that name the app on its notifications, written into the
    /// desktop entry `install` keeps for the app and passed to the notification server. The icon
    /// is an absolute path to an image or an icon theme's name. Without them the notifications
    /// carry the app id and no app icon.
    #[must_use]
    pub fn app_notifications(
        self,
        display_name: impl Into<String>,
        icon: impl Into<String>,
    ) -> Self {
        Self {
            notifications: Some(AppNotifications {
                display_name: display_name.into(),
                icon: icon.into(),
            }),
            ..self
        }
    }
}

/// The WNS part of [`Config`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WnsConfig {
    pub(crate) remote_id: String,
    pub(crate) notifications: Option<AppNotifications>,
}

/// The display name and icon an app shows its notifications under, what
/// [`WnsConfig::app_notifications`] and [`LinuxConfig::app_notifications`] store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppNotifications {
    pub(crate) display_name: String,
    pub(crate) icon: String,
}

impl WnsConfig {
    /// A configuration for the Object ID of the app's Entra ID registration, the `remoteId`
    /// that `CreateChannelAsync` takes.
    #[must_use]
    pub fn new(remote_id: impl Into<String>) -> Self {
        Self {
            remote_id: remote_id.into(),
            notifications: None,
        }
    }

    /// The display name and icon `install` registers with the App SDK's notification manager,
    /// which [`Notification::show`](crate::Notification::show) needs before it can show a
    /// notification, and reports [`Error::NotConfigured`](crate::Error::NotConfigured) without.
    #[must_use]
    pub fn app_notifications(
        self,
        display_name: impl Into<String>,
        icon: impl Into<String>,
    ) -> Self {
        Self {
            notifications: Some(AppNotifications {
                display_name: display_name.into(),
                icon: icon.into(),
            }),
            ..self
        }
    }
}

/// The Web Push part of [`Config`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebPushConfig {
    pub(crate) vapid_public_key: VapidPublicKey,
    pub(crate) service_worker_path: Option<String>,
    pub(crate) rust_handler: bool,
}

impl WebPushConfig {
    /// The path of the static service worker unless [`service_worker_path`](Self::service_worker_path)
    /// sets another. With the `dioxus` feature the default is the copy `dx` bundles from this
    /// crate into the app's assets.
    pub const DEFAULT_SERVICE_WORKER_PATH: &str = "/pushups-sw.js";

    /// A configuration for the app's VAPID public key.
    #[must_use]
    pub fn new(vapid_public_key: impl Into<VapidPublicKey>) -> Self {
        Self {
            vapid_public_key: vapid_public_key.into(),
            service_worker_path: None,
            rust_handler: false,
        }
    }

    /// Where the app serves the static worker, [`SERVICE_WORKER`](crate::SERVICE_WORKER). With
    /// [`rust_handler`](Self::rust_handler) it is where the app serves its own worker entry, a
    /// module that imports the app's wasm-bindgen glue and calls its `init`.
    #[must_use]
    pub fn service_worker_path(mut self, path: impl Into<String>) -> Self {
        self.service_worker_path = Some(path.into());
        self
    }

    /// Runs the app's own wasm in the service worker, so its `main` can serve a Rust handler
    /// through [`serve_service_worker`](crate::serve_service_worker).
    ///
    /// Without a [`service_worker_path`](Self::service_worker_path) the page registers the
    /// app's wasm-bindgen glue itself, which starts the wasm only when it initializes itself, as
    /// `dx`'s does. Under Trunk, cargo-leptos or wasm-pack, whose glue the page initializes, the
    /// app serves a worker entry and sets its path.
    #[must_use]
    pub fn rust_handler(mut self) -> Self {
        self.rust_handler = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;

    use super::*;

    const P256DH: [u8; 65] = [
        0x04, 0x25, 0x71, 0xb2, 0xbe, 0xcd, 0xfd, 0xe3, 0x60, 0x55, 0x1a, 0xaf, 0x1e, 0xd0, 0xf4,
        0xcd, 0x36, 0x6c, 0x11, 0xce, 0xbe, 0x55, 0x5f, 0x89, 0xbc, 0xb7, 0xb1, 0x86, 0xa5, 0x33,
        0x39, 0x17, 0x31, 0x68, 0xec, 0xe2, 0xeb, 0xe0, 0x18, 0x59, 0x7b, 0xd3, 0x04, 0x79, 0xb8,
        0x6e, 0x3c, 0x8f, 0x8e, 0xce, 0xd5, 0x77, 0xca, 0x59, 0x18, 0x7e, 0x92, 0x46, 0x99, 0x0d,
        0xb6, 0x82, 0x00, 0x8b, 0x0e,
    ];

    const P256DH_BASE64URL: &str =
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";

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
}
