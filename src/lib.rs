#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[doc(hidden)]
#[path = "private.rs"]
pub mod __private;
#[cfg(target_os = "android")]
mod android;
mod android_context;
mod config;
mod dispatch;
mod error;
mod event;
mod notification;
mod permission;
#[cfg(any(target_os = "android", test))]
mod queue;
#[cfg(any(target_os = "android", test))]
mod session;
mod token;
#[cfg(not(any(
    target_os = "android",
    all(target_arch = "wasm32", target_os = "unknown")
)))]
mod unsupported;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod web;
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
mod web_parts;

use std::sync::Arc;

#[cfg(target_os = "android")]
use android as platform;
#[cfg(not(any(
    target_os = "android",
    all(target_arch = "wasm32", target_os = "unknown")
)))]
use unsupported as platform;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use web as platform;

pub use android_context::AndroidContext;
pub use config::{Config, WebPushConfig};
use dispatch::DISPATCHER;
pub use error::Error;
pub use event::{Event, Message};
pub use notification::Notification;
pub use permission::Permission;
pub use pushups_macros::{background_handler, firebase_config};
pub use token::{Token, WebPushKeys};
/// Connects the app to the platform's push service, before the toolkit's event loop starts.
///
/// # Errors
///
/// [`Error::Unsupported`] on a target without a push backend or a browser without the Push API,
/// [`Error::AndroidModuleMissing`] on Android when the app does not include the crate's Gradle
/// module, and [`Error::NotConfigured`] on the web without a [`WebPushConfig`].
pub fn install(config: Config) -> Result<(), Error> {
    platform::install(config)
}

/// Asks the platform for a push token, which arrives later as [`Event::Token`], or as
/// [`Event::RegistrationFailed`].
///
/// # Errors
///
/// The errors of [`install`], and [`Error::Platform`] when the request cannot be sent.
pub fn register() -> Result<(), Error> {
    platform::register()
}

/// Asks the user to let pushes show notifications, and returns the answer.
///
/// # Errors
///
/// The errors of [`install`], and [`Error::Platform`] when the prompt cannot be shown or ends
/// without an answer.
pub async fn request_permission() -> Result<Permission, Error> {
    platform::request_permission().await
}

/// Sets the function that receives every [`Event`], replacing the previous one.
///
/// Events that arrive before the first handler is set, such as the push that started the app,
/// are kept and delivered to that first handler, in the order they arrived. On Android the
/// pushes that arrived while the app had no UI wait on disk and go to the first handler of the
/// next UI session. The handler runs on whichever thread delivers the event, one event at a
/// time and never concurrently with itself. Waking the toolkit's event loop, through a tao or
/// winit `EventLoopProxy` for instance, is the handler's job.
pub fn set_handler(handler: impl Fn(Event) + Send + Sync + 'static) {
    DISPATCHER.set_handler(Arc::new(handler));
    platform::handler_set();
}

/// Returns every [`Event`] as a stream, in place of a handler.
///
/// The stream is the handler: it receives the events kept from before the first handler, and it
/// ends when [`set_handler`] or another call to `events` replaces it. Events arriving after the
/// stream is dropped and before another handler is set are discarded.
#[cfg(feature = "stream")]
#[cfg_attr(docsrs, doc(cfg(feature = "stream")))]
pub fn events() -> impl futures_core::Stream<Item = Event> + Send + Unpin {
    let events = DISPATCHER.events();
    platform::handler_set();
    events
}

/// Whether this code runs in the app's service worker, where `main` should call
/// [`serve_service_worker`] instead of starting its UI.
#[must_use]
pub fn in_service_worker() -> bool {
    platform::in_service_worker()
}

/// Builds the notification for every push the service worker receives, from the app's own
/// Rust, for an app that set [`WebPushConfig::rust_handler`].
///
/// # Errors
///
/// [`Error::NotInServiceWorker`] outside the service worker.
pub fn serve_service_worker<H, F>(handler: H) -> Result<(), Error>
where
    H: Fn(Message) -> F + 'static,
    F: Future<Output = Notification> + 'static,
{
    platform::serve_service_worker(handler)
}
