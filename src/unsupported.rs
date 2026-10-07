//! Targets without a push backend.

use std::future::{Ready, ready};

use crate::{Config, Error, Message, Notification, Permission};

pub(crate) fn install(_config: Config) -> Result<(), Error> {
    Err(Error::Unsupported)
}

pub(crate) fn register() -> Result<(), Error> {
    Err(Error::Unsupported)
}

pub(crate) fn request_permission() -> Ready<Result<Permission, Error>> {
    ready(Err(Error::Unsupported))
}

/// Nothing outside the dispatcher keeps events on these targets.
pub(crate) fn handler_set() {}

pub(crate) fn in_service_worker() -> bool {
    false
}

pub(crate) fn serve_service_worker<H, F>(_handler: H) -> Result<(), Error>
where
    H: Fn(Message) -> F + 'static,
    F: Future<Output = Notification> + 'static,
{
    Err(Error::Unsupported)
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
        assert_eq!(install(Config::new()), Err(Error::Unsupported));
        assert_eq!(register(), Err(Error::Unsupported));
        assert_eq!(
            futures_executor::block_on(request_permission()),
            Err(Error::Unsupported)
        );
        handler_set();
        assert!(!in_service_worker());
        assert_eq!(
            serve_service_worker(notification_for),
            Err(Error::Unsupported)
        );
    }
}
