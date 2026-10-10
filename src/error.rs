/// An error reported by the platform's push service or by `pushups`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The platform push service refused or failed the request, with its own description.
    #[error("push service error: {0}")]
    Platform(String),
    /// `pushups` has no push backend for this target.
    #[error("push notifications are not supported on this target")]
    Unsupported,
    /// The Gradle module `pushups` ships in `android/` is not part of the app, so its
    /// `ContentProvider` never handed the process to Rust.
    #[error("the pushups Android module is not in the app")]
    AndroidModuleMissing,
    /// The app's Gradle module is another version of `pushups` than its Rust crate, such as an
    /// AAR of one release beside a crate of another, so the module handed nothing to Rust.
    #[error(
        "the pushups Android module is version {module}, but the app's pushups crate is {crate_version}"
    )]
    AndroidModuleMismatch {
        /// The version the Gradle module was released with.
        module: String,
        /// The version of the `pushups` crate in the app.
        crate_version: String,
    },
    /// The target's part of [`Config`](crate::Config) is missing, a
    /// [`WebPushConfig`](crate::WebPushConfig) on the web, a [`LinuxConfig`](crate::LinuxConfig)
    /// on Linux or a [`WnsConfig`](crate::WnsConfig) on Windows.
    #[error("Config lacks the part this target needs")]
    NotConfigured,
    /// [`serve_service_worker`](crate::serve_service_worker) ran in a web page, outside the
    /// service worker.
    #[error("not running in a service worker")]
    NotInServiceWorker,
}
