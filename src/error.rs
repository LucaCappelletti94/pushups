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
}
