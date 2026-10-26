//! What every backend provides to the public functions, one implementation compiled per target.

use crate::{Config, Error, Message, Notification, Permission};

pub(crate) trait Backend {
    fn install(config: Config) -> Result<(), Error>;

    fn register() -> Result<(), Error>;

    fn request_permission() -> impl Future<Output = Result<Permission, Error>>;

    fn permission_status() -> impl Future<Output = Result<Permission, Error>>;

    /// A handler was set, so events kept outside the dispatcher go to it now.
    fn handler_set() {}

    fn in_service_worker() -> bool {
        false
    }

    /// Replaces the stored keys with fresh ones and registers again, so the new keys arrive as
    /// an [`Event::Token`](crate::Event::Token).
    #[cfg(feature = "encryption")]
    fn rotate_keys() -> Result<(), Error> {
        Err(Error::Unsupported)
    }

    fn serve_service_worker<H, F>(_handler: H) -> Result<(), Error>
    where
        H: Fn(Message) -> F + 'static,
        F: Future<Output = Notification> + 'static,
    {
        Err(Error::Unsupported)
    }
}
