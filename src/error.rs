/// An error reported by the platform's push service or by `pushups`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The platform push service refused or failed the request, with its own description.
    #[error("push service error: {0}")]
    Platform(String),
}
