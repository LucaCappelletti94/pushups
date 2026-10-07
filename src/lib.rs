#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/brand/icon.svg",
    html_favicon_url = "https://raw.githubusercontent.com/LucaCappelletti94/pushups/main/assets/brand/favicon.svg"
)]

#[doc(hidden)]
#[path = "private.rs"]
pub mod __private;
#[cfg(target_os = "android")]
mod android;
#[cfg(any(target_os = "ios", target_os = "macos"))]
mod apple;
mod config;
mod context;
#[cfg(any(target_os = "ios", target_os = "macos", test))]
mod delivered;
mod dispatch;
mod error;
mod event;
#[cfg(any(
    target_os = "android",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos"
))]
mod inbox;
#[cfg(target_os = "linux")]
mod linux;
mod notification;
mod permission;
#[cfg(any(
    target_os = "android",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos",
    test
))]
mod queue;
#[cfg(any(
    target_os = "android",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos",
    test
))]
mod session;
#[cfg(any(target_os = "ios", target_os = "macos", test))]
mod started;
mod token;
#[cfg(any(
    not(any(
        target_os = "android",
        target_os = "ios",
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        all(target_arch = "wasm32", target_os = "unknown")
    )),
    test
))]
mod unsupported;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod web;
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
mod web_parts;
#[cfg(target_os = "linux")]
mod webpush_crypto;
#[cfg(any(target_os = "windows", test))]
mod windows;

use std::sync::Arc;

#[cfg(target_os = "android")]
use android as platform;
#[cfg(any(target_os = "ios", target_os = "macos"))]
use apple as platform;
#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos",
    target_os = "windows",
    all(target_arch = "wasm32", target_os = "unknown")
)))]
use unsupported as platform;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use web as platform;
#[cfg(target_os = "windows")]
use windows as platform;

pub use config::{Config, LinuxConfig, UnifiedPushConfig, WebPushConfig, WnsConfig};
pub use context::Context;
use dispatch::DISPATCHER;
pub use error::Error;
pub use event::{Event, Message};
pub use notification::Notification;
pub use permission::Permission;
pub use pushups_macros::{background_handler, firebase_config};
pub use token::{Token, WebPushKeys};
/// Connects the app to the platform's push service, before the toolkit's event loop starts.
///
/// On Linux a process the session bus started for a push, through the activation file `install`
/// writes, never returns from `install`. It persists the pushes, runs the
/// [`background_handler`], and exits once idle, so the app's window never opens for it.
///
/// # Errors
///
/// [`Error::Unsupported`] on a target without a push backend, a browser without the Push API,
/// or Windows without the Windows App SDK push API. [`Error::AndroidModuleMissing`] on Android
/// when the app does not include the crate's Gradle module. [`Error::NotConfigured`] on the
/// web without a [`WebPushConfig`], on Linux without a [`LinuxConfig`] and on Windows without
/// a [`WnsConfig`]. [`Error::Platform`] on Linux without a session bus or when another process
/// of the app owns its bus name, and on Windows when the `remote_id` is not a GUID or the
/// runtime does not start.
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
/// [`Error::Unsupported`] off the web, and [`Error::NotInServiceWorker`] in a web page.
pub fn serve_service_worker<H, F>(handler: H) -> Result<(), Error>
where
    H: Fn(Message) -> F + 'static,
    F: Future<Output = Notification> + 'static,
{
    platform::serve_service_worker(handler)
}
