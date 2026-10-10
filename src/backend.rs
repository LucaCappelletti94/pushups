//! What every backend provides to the public functions, one implementation compiled per target.

use crate::{Config, Error, Message, Notification, Permission};

pub(crate) trait Backend {
    fn install(config: Config) -> Result<(), Error>;

    fn register() -> Result<(), Error>;

    fn request_permission() -> impl Future<Output = Result<Permission, Error>>;

    /// A handler was set, so events kept outside the dispatcher go to it now.
    fn handler_set() {}

    fn in_service_worker() -> bool {
        false
    }

    fn serve_service_worker<H, F>(_handler: H) -> Result<(), Error>
    where
        H: Fn(Message) -> F + 'static,
        F: Future<Output = Notification> + 'static,
    {
        Err(Error::Unsupported)
    }
}
