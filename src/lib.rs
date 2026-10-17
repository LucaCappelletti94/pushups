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
mod backend;
mod config;
mod context;
#[cfg(any(
    target_os = "ios",
    target_os = "macos",
    target_os = "linux",
    target_os = "windows",
    test
))]
mod delivered;
mod dispatch;
#[cfg(all(
    feature = "dioxus",
    any(
        target_os = "android",
        all(target_arch = "wasm32", target_os = "unknown")
    )
))]
mod dx;
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
mod worker_file;

use std::sync::Arc;

#[cfg(target_os = "android")]
use android::Android as Platform;
#[cfg(any(target_os = "ios", target_os = "macos"))]
use apple::Apple as Platform;
use backend::Backend as _;
#[cfg(target_os = "linux")]
use linux::Linux as Platform;
#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos",
    target_os = "windows",
    all(target_arch = "wasm32", target_os = "unknown")
)))]
use unsupported::Unsupported as Platform;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use web::Web as Platform;
#[cfg(target_os = "windows")]
use windows::Windows as Platform;

pub use config::{
    Config, LinuxConfig, UnifiedPushConfig, VapidPublicKey, WebPushConfig, WnsConfig,
};
pub use context::Context;
use dispatch::DISPATCHER;
pub use error::{Error, VapidKeyError};
pub use event::{Event, Message};
pub use notification::Notification;
pub use permission::Permission;
pub use pushups_macros::{background_handler, firebase_config};
pub use token::{Token, WebPushKeys, WebPushSubscription};
pub use worker_file::write_service_worker;

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
/// when the app does not include the crate's Gradle module, and [`Error::AndroidModuleMismatch`]
/// when it includes another version of it. [`Error::NotConfigured`] on the web without a
/// [`WebPushConfig`], on Linux without a [`LinuxConfig`] and on Windows without
/// a [`WnsConfig`]. [`Error::Platform`] on Linux without a session bus or when another process
/// of the app owns its bus name, and on Windows when the `remote_id` is not a GUID or the
/// runtime does not start.
pub fn install(config: Config) -> Result<(), Error> {
    Platform::install(config)
}

/// Asks the platform for a push token, which arrives later as [`Event::Token`], or as
/// [`Event::RegistrationFailed`].
///
/// # Errors
///
/// The errors of [`install`], and [`Error::Platform`] when the request cannot be sent.
pub fn register() -> Result<(), Error> {
    Platform::register()
}

/// Asks the user to let pushes show notifications, and returns the answer.
///
/// On Linux and Windows it reports transport readiness, the session bus and a distributor on
/// Linux and `PushNotificationManager::IsSupported` on Windows, and the notification setting is
/// what [`permission_status`] reports.
///
/// On the web a grant subscribes again when the browser refused an earlier [`register`] for want
/// of the permission, as Safari refuses one outside a user gesture, so the token still arrives
/// as [`Event::Token`].
///
/// Off the web the future is `Send`, so a multi-threaded executor such as Tauri's can await it.
///
/// # Errors
///
/// The errors of [`install`], and [`Error::Platform`] when the prompt cannot be shown or ends
/// without an answer.
pub async fn request_permission() -> Result<Permission, Error> {
    Platform::request_permission().await
}

/// Reports the notification permission the platform holds, without asking the user.
///
/// Linux has no notification permission, so its answer is [`Permission::Granted`] once
/// [`install`] returned, and Windows reports the App SDK's `AppNotificationManager.Setting`.
///
/// Off the web the future is `Send`, so a multi-threaded executor such as Tauri's can await it.
///
/// # Errors
///
/// The errors of [`install`], and [`Error::Platform`] when the platform cannot answer.
pub async fn permission_status() -> Result<Permission, Error> {
    Platform::permission_status().await
}

// Holds `request_permission` and `permission_status` to their documented `Send` futures on every
// native target.
#[cfg(not(target_arch = "wasm32"))]
const _: fn() = || {
    fn send<T: Send>(_: &T) {}
    send(&request_permission());
    send(&permission_status());
};

/// Sets the function that receives every [`Event`], replacing the previous one.
///
/// Events that arrive before the first handler is set, such as the push that started the app,
/// are kept and delivered to that first handler, in the order they arrived. On Android the
/// pushes that arrived while the app had no UI wait on disk for the next UI session. They go to
/// the handler already set if the thread that set it still runs, as a toolkit whose event loop
/// outlives its Activity keeps it, and otherwise to the next handler set. The handler runs on
/// whichever thread delivers the event, one event at a time and never concurrently with itself.
/// Waking the toolkit's event loop, through a tao or winit `EventLoopProxy` for instance, is the
/// handler's job.
pub fn set_handler(handler: impl Fn(Event) + Send + Sync + 'static) {
    DISPATCHER.set_handler(Arc::new(handler));
    Platform::handler_set();
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
    Platform::handler_set();
    events
}

/// Whether this code runs in the app's service worker, where `main` should call
/// [`serve_service_worker`] instead of starting its UI.
#[must_use]
pub fn in_service_worker() -> bool {
    Platform::in_service_worker()
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
    Platform::serve_service_worker(handler)
}

/// The source of the static service worker, `js/pushups-sw.js`, for an app whose bundler does
/// not copy it out of the crate as `dx` does with the `dioxus` feature.
///
/// The app serves it as JavaScript at [`WebPushConfig::service_worker_path`], `/pushups-sw.js`
/// by default, from a route of its own server or as a file its build writes. It is the same on
/// every target, so a server built for the host serves the worker its wasm registers.
pub const SERVICE_WORKER: &str = include_str!("../js/pushups-sw.js");
