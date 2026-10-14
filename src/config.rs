use crate::Error;
use crate::token::base64url_decode_vapid_key;

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
        crate::token::base64url(&self.0)
    }
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
        }
    }
}

/// The WNS part of [`Config`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WnsConfig {
    pub(crate) remote_id: String,
}

impl WnsConfig {
    /// A configuration for the Object ID of the app's Entra ID registration, the `remoteId`
    /// that `CreateChannelAsync` takes.
    #[must_use]
    pub fn new(remote_id: impl Into<String>) -> Self {
        Self {
            remote_id: remote_id.into(),
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
