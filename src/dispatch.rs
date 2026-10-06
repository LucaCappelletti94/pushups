use std::collections::VecDeque;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;

use parking_lot::{Mutex, MutexGuard};

use crate::Event;

pub(crate) type Handler = dyn Fn(Event) + Send + Sync;

/// The dispatcher every platform backend emits into.
pub(crate) static DISPATCHER: Dispatcher = Dispatcher::new();

/// Hands events to the current handler one at a time, in the order they were emitted.
///
/// Events wait in the queue until a handler is set. The thread that finds events queued while
/// nobody is delivering delivers until the queue is empty, so the handler never runs
/// concurrently with itself, and an event emitted during a delivery, by the handler or by any
/// other thread, queues behind the events emitted before it. A handler that panics loses only
/// the event it was given, and the panic reaches the delivering caller once the queue is empty.
pub(crate) struct Dispatcher {
    state: Mutex<State>,
}

struct State {
    handler: Option<Arc<Handler>>,
    queue: VecDeque<Event>,
    delivering: bool,
}

impl Dispatcher {
    pub(crate) const fn new() -> Self {
        Self {
            state: Mutex::new(State {
                handler: None,
                queue: VecDeque::new(),
                delivering: false,
            }),
        }
    }

    /// Replaces the handler and delivers any queued events to it.
    pub(crate) fn set_handler(&self, handler: Arc<Handler>) {
        let mut state = self.state.lock();
        state.handler = Some(handler);
        self.deliver(state);
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "platform backends emit events, and this target has none"
        )
    )]
    pub(crate) fn emit(&self, event: Event) {
        let mut state = self.state.lock();
        state.queue.push_back(event);
        self.deliver(state);
    }

    /// Replaces the handler with one feeding the returned receiver.
    #[cfg(feature = "stream")]
    pub(crate) fn events(&self) -> futures_channel::mpsc::UnboundedReceiver<Event> {
        let (sender, receiver) = futures_channel::mpsc::unbounded();
        self.set_handler(Arc::new(move |event| {
            // A dropped receiver means the app stopped listening, so its events have nowhere to go.
            let _ = sender.unbounded_send(event);
        }));
        receiver
    }

    fn deliver<'a>(&'a self, mut state: MutexGuard<'a, State>) {
        if state.delivering {
            return;
        }
        state.delivering = true;
        let mut first_panic = None;
        loop {
            let Some(handler) = state.handler.clone() else {
                break;
            };
            let Some(event) = state.queue.pop_front() else {
                break;
            };
            drop(state);
            if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(|| handler(event))) {
                first_panic.get_or_insert(payload);
            }
            state = self.state.lock();
        }
        state.delivering = false;
        drop(state);
        if let Some(payload) = first_panic {
            panic::resume_unwind(payload);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Message, Token};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::thread;

    fn message(byte: u8) -> Event {
        Event::Message(Message {
            payload: vec![byte],
            started_app: false,
        })
    }

    fn recorder() -> (Arc<Mutex<Vec<Event>>>, Arc<Handler>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        (seen, Arc::new(move |event| sink.lock().push(event)))
    }

    #[test]
    fn events_before_the_handler_are_replayed_in_order_then_live_events_follow() {
        let dispatcher = Dispatcher::new();
        dispatcher.emit(Event::Token(Token::Fcm("t".to_owned())));
        dispatcher.emit(message(1));
        let (seen, handler) = recorder();
        dispatcher.set_handler(handler);
        dispatcher.emit(message(2));
        assert_eq!(
            *seen.lock(),
            [
                Event::Token(Token::Fcm("t".to_owned())),
                message(1),
                message(2)
            ]
        );
    }

    #[test]
    fn a_replacing_handler_gets_only_later_events() {
        let dispatcher = Dispatcher::new();
        dispatcher.emit(message(1));
        let (first, handler) = recorder();
        dispatcher.set_handler(handler);
        let (second, handler) = recorder();
        dispatcher.set_handler(handler);
        dispatcher.emit(message(2));
        assert_eq!(*first.lock(), [message(1)]);
        assert_eq!(*second.lock(), [message(2)]);
    }

    #[test]
    fn an_event_from_another_thread_during_replay_waits_for_the_replay() {
        let dispatcher = Arc::new(Dispatcher::new());
        dispatcher.emit(message(1));
        dispatcher.emit(message(2));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let handler = {
            let seen = Arc::clone(&seen);
            let dispatcher = Arc::clone(&dispatcher);
            move |event: Event| {
                if event == message(1) {
                    let dispatcher = Arc::clone(&dispatcher);
                    thread::spawn(move || dispatcher.emit(message(3)))
                        .join()
                        .unwrap();
                }
                seen.lock().push(event);
            }
        };
        dispatcher.set_handler(Arc::new(handler));
        assert_eq!(*seen.lock(), [message(1), message(2), message(3)]);
    }

    #[test]
    fn a_panicking_handler_loses_only_its_event() {
        let dispatcher = Dispatcher::new();
        dispatcher.emit(message(1));
        dispatcher.emit(message(2));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let handler = {
            let seen = Arc::clone(&seen);
            move |event: Event| {
                assert_ne!(event, message(1), "handler failure");
                seen.lock().push(event);
            }
        };
        let replay = catch_unwind(AssertUnwindSafe(|| {
            dispatcher.set_handler(Arc::new(handler));
        }));
        assert!(replay.is_err());
        dispatcher.emit(message(3));
        assert_eq!(*seen.lock(), [message(2), message(3)]);
    }

    #[test]
    fn events_queued_while_a_handler_panics_are_still_delivered() {
        let dispatcher = Arc::new(Dispatcher::new());
        dispatcher.emit(message(1));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let handler = {
            let seen = Arc::clone(&seen);
            let dispatcher = Arc::clone(&dispatcher);
            move |event: Event| {
                if event == message(1) {
                    dispatcher.emit(message(2));
                    panic!("handler failure");
                }
                seen.lock().push(event);
            }
        };
        let replay = catch_unwind(AssertUnwindSafe(|| {
            dispatcher.set_handler(Arc::new(handler));
        }));
        assert!(replay.is_err());
        assert_eq!(*seen.lock(), [message(2)]);
    }

    #[test]
    fn concurrent_emitters_lose_nothing_and_keep_their_own_order() {
        const THREADS: u8 = 8;
        const PER_THREAD: u8 = 200;
        let dispatcher = Arc::new(Dispatcher::new());
        let (seen, handler) = recorder();
        let emitters: Vec<_> = (0..THREADS)
            .map(|thread_index| {
                let dispatcher = Arc::clone(&dispatcher);
                thread::spawn(move || {
                    for sequence in 0..PER_THREAD {
                        dispatcher.emit(Event::Message(Message {
                            payload: vec![thread_index, sequence],
                            started_app: false,
                        }));
                    }
                })
            })
            .collect();
        dispatcher.set_handler(handler);
        for emitter in emitters {
            emitter.join().unwrap();
        }
        let seen = seen.lock();
        assert_eq!(seen.len(), usize::from(THREADS) * usize::from(PER_THREAD));
        for thread_index in 0..THREADS {
            let sequences: Vec<u8> = seen
                .iter()
                .filter_map(|event| match event {
                    Event::Message(message) if message.payload[0] == thread_index => {
                        Some(message.payload[1])
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(sequences, (0..PER_THREAD).collect::<Vec<_>>());
        }
    }

    #[cfg(feature = "stream")]
    #[test]
    fn the_stream_yields_replayed_then_live_events_and_ends_when_replaced() {
        let dispatcher = Dispatcher::new();
        dispatcher.emit(message(1));
        let events = dispatcher.events();
        dispatcher.emit(message(2));
        dispatcher.set_handler(Arc::new(|_| {}));
        let received: Vec<Event> = futures_executor::block_on_stream(events).collect();
        assert_eq!(received, [message(1), message(2)]);
    }
}
