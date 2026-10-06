/// What [`install`](crate::install) needs beyond the platform's own configuration.
///
/// Each push service that needs values from the app gets its own part, set with its own
/// method, so the same setup code compiles on every target. Apple and Android need none:
/// Android reads Firebase's values through [`firebase_config!`](crate::firebase_config).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {}

impl Config {
    /// A configuration with no service-specific part.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}
