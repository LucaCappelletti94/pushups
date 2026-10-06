#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod dispatch;
mod error;
mod event;
mod token;

use std::sync::Arc;

use dispatch::DISPATCHER;
pub use error::Error;
pub use event::{Event, Message};
pub use token::{Token, WebPushKeys};

/// Sets the function that receives every [`Event`], replacing the previous one.
///
/// Events that arrive before the first handler is set, such as the push that started the app,
/// are kept and delivered to that first handler, in the order they arrived. The handler runs on
/// whichever thread delivers the event, one event at a time and never concurrently with itself.
/// Waking the toolkit's event loop, through a tao or winit `EventLoopProxy` for instance, is the
/// handler's job.
pub fn set_handler(handler: impl Fn(Event) + Send + Sync + 'static) {
    DISPATCHER.set_handler(Arc::new(handler));
}

/// Returns every [`Event`] as a stream, in place of a handler.
///
/// The stream is the handler: it receives the events kept from before the first handler, and it
/// ends when [`set_handler`] or another call to `events` replaces it. Events arriving after the
/// stream is dropped and before another handler is set are discarded.
#[cfg(feature = "stream")]
#[cfg_attr(docsrs, doc(cfg(feature = "stream")))]
pub fn events() -> impl futures_core::Stream<Item = Event> + Send + Unpin {
    DISPATCHER.events()
}
