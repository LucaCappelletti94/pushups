//! Targets without a push backend.

use std::future::ready;

use crate::backend::Backend;
use crate::{Config, Error, Permission};

pub(crate) struct Unsupported;

impl Backend for Unsupported {
    fn install(_config: Config) -> Result<(), Error> {
        Err(Error::Unsupported)
    }

    fn register() -> Result<(), Error> {
        Err(Error::Unsupported)
    }

    fn request_permission() -> impl Future<Output = Result<Permission, Error>> {
        ready(Err(Error::Unsupported))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Error, Message, Notification};

    async fn notification_for(_message: Message) -> Notification {
        Notification::new("never shown")
    }

    #[test]
    fn a_target_without_a_backend_refuses_everything() {
        assert_eq!(Unsupported::install(Config::new()), Err(Error::Unsupported));
        assert_eq!(Unsupported::register(), Err(Error::Unsupported));
        assert_eq!(
            futures_executor::block_on(Unsupported::request_permission()),
            Err(Error::Unsupported)
        );
        Unsupported::handler_set();
        assert!(!Unsupported::in_service_worker());
        assert_eq!(
            Unsupported::serve_service_worker(notification_for),
            Err(Error::Unsupported)
        );
    }
}
