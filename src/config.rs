/// What [`install`](crate::install) needs beyond the platform's own configuration.
///
/// Each push service that needs values from the app gets its own part, set with its own
/// method, so the same setup code compiles on every target. Apple and Android need none:
/// Android reads Firebase's values through [`firebase_config!`](crate::firebase_config).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub(crate) web: Option<WebPushConfig>,
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
}

/// The Web Push part of [`Config`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebPushConfig {
    pub(crate) vapid_public_key: [u8; 65],
    pub(crate) service_worker_path: String,
    pub(crate) rust_handler: bool,
}

impl WebPushConfig {
    /// The service worker path used unless [`service_worker_path`](Self::service_worker_path)
    /// sets another.
    pub const DEFAULT_SERVICE_WORKER_PATH: &str = "/pushups-sw.js";

    /// A configuration for the app's VAPID public key, an uncompressed P-256 point.
    #[must_use]
    pub fn new(vapid_public_key: [u8; 65]) -> Self {
        Self {
            vapid_public_key,
            service_worker_path: Self::DEFAULT_SERVICE_WORKER_PATH.to_owned(),
            rust_handler: false,
        }
    }

    /// Where the app serves the `js/pushups-sw.js` file this crate ships.
    #[must_use]
    pub fn service_worker_path(mut self, path: impl Into<String>) -> Self {
        self.service_worker_path = path.into();
        self
    }

    /// Registers the app's own wasm as the service worker, so its `main` can serve a Rust
    /// handler through [`serve_service_worker`](crate::serve_service_worker).
    #[must_use]
    pub fn rust_handler(mut self) -> Self {
        self.rust_handler = true;
        self
    }
}
