use std::collections::VecDeque;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
#[cfg(any(target_os = "android", test))]
use std::sync::atomic::{AtomicBool, Ordering};

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
    /// Cleared when the thread that set `handler` exits.
    #[cfg(any(target_os = "android", test))]
    setter_alive: Option<Arc<AtomicBool>>,
    queue: VecDeque<Event>,
    delivering: bool,
}

/// A flag per thread, cleared by its thread-local destructor when the thread exits.
#[cfg(any(target_os = "android", test))]
struct ThreadAlive(Arc<AtomicBool>);

#[cfg(any(target_os = "android", test))]
impl Drop for ThreadAlive {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[cfg(any(target_os = "android", test))]
thread_local! {
    static THREAD_ALIVE: ThreadAlive = ThreadAlive(Arc::new(AtomicBool::new(true)));
}

/// The calling thread's flag. A thread already tearing down its thread-locals counts as exited.
#[cfg(any(target_os = "android", test))]
fn current_thread_alive() -> Arc<AtomicBool> {
    THREAD_ALIVE
        .try_with(|alive| Arc::clone(&alive.0))
        .unwrap_or_else(|_| Arc::new(AtomicBool::new(false)))
}

impl Dispatcher {
    pub(crate) const fn new() -> Self {
        Self {
            state: Mutex::new(State {
                handler: None,
                #[cfg(any(target_os = "android", test))]
                setter_alive: None,
                queue: VecDeque::new(),
                delivering: false,
            }),
        }
    }

    /// Replaces the handler and delivers any queued events to it.
    pub(crate) fn set_handler(&self, handler: Arc<Handler>) {
        let mut state = self.state.lock();
        // Dropped once the lock is released, since the handler's captures may call back in.
        let _replaced = state.handler.replace(handler);
        #[cfg(any(target_os = "android", test))]
        {
            state.setter_alive = Some(current_thread_alive());
        }
        self.deliver(state);
    }

    /// Whether a handler is set and the thread that set it still runs.
    #[cfg(any(target_os = "android", test))]
    pub(crate) fn handler_thread_alive(&self) -> bool {
        self.state
            .lock()
            .setter_alive
            .as_ref()
            .is_some_and(|alive| alive.load(Ordering::Acquire))
    }

    #[cfg_attr(
        not(any(
            test,
            target_os = "android",
            target_os = "ios",
            target_os = "linux",
            target_os = "macos",
            target_os = "windows",
            all(target_arch = "wasm32", target_os = "unknown")
        )),
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

    /// Queues `event` behind the ones before it without delivering, for a caller holding a lock
    /// the handler may need. [`flush`](Self::flush) delivers.
    #[cfg_attr(
        not(any(
            test,
            target_os = "android",
            target_os = "ios",
            target_os = "linux",
            target_os = "macos"
        )),
        expect(
            dead_code,
            reason = "the Android, Apple and Linux backends queue under their own lock, and this target has none"
        )
    )]
    pub(crate) fn enqueue(&self, event: Event) {
        self.state.lock().queue.push_back(event);
    }

    /// Delivers the queued events, if a handler is set.
    #[cfg_attr(
        not(any(
            test,
            target_os = "android",
            target_os = "ios",
            target_os = "linux",
            target_os = "macos"
        )),
        expect(
            dead_code,
            reason = "the Android, Apple and Linux backends queue under their own lock, and this target has none"
        )
    )]
    pub(crate) fn flush(&self) {
        self.deliver(self.state.lock());
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
            // Dropping a handler that replaced itself, whose last reference this is, runs the app's
            // destructors, so it happens before the lock is retaken and a panic in them is caught
            // like one in the call.
            if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(move || {
                handler(event);
                drop(handler);
            })) {
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
        dispatcher.emit(Event::Token(Token::Fcm {
            token: "t".to_owned(),
            keys: None,
        }));
        dispatcher.emit(message(1));
        let (seen, handler) = recorder();
        dispatcher.set_handler(handler);
        dispatcher.emit(message(2));
        assert_eq!(
            *seen.lock(),
            [
                Event::Token(Token::Fcm {
                    token: "t".to_owned(),
                    keys: None
                }),
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

    #[test]
    fn enqueued_events_wait_for_flush_and_keep_their_place() {
        let dispatcher = Dispatcher::new();
        let (seen, handler) = recorder();
        dispatcher.set_handler(handler);
        dispatcher.enqueue(message(1));
        dispatcher.enqueue(message(2));
        assert!(seen.lock().is_empty());
        dispatcher.emit(message(3));
        assert_eq!(*seen.lock(), [message(1), message(2), message(3)]);
        dispatcher.enqueue(message(4));
        dispatcher.flush();
        assert_eq!(seen.lock().len(), 4);
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

    #[test]
    fn the_handler_reports_whether_the_thread_that_set_it_still_runs() {
        let dispatcher = Arc::new(Dispatcher::new());
        assert!(!dispatcher.handler_thread_alive(), "no handler is set");
        let (_, handler) = recorder();
        dispatcher.set_handler(handler);
        assert!(dispatcher.handler_thread_alive());
        let setter = Arc::clone(&dispatcher);
        thread::spawn(move || setter.set_handler(recorder().1))
            .join()
            .unwrap();
        assert!(!dispatcher.handler_thread_alive());
    }

    /// Emits `message(byte)` into `dispatcher` when dropped, as a handler's captured state may.
    struct EmitOnDrop {
        dispatcher: Arc<Dispatcher>,
        byte: u8,
    }

    impl Drop for EmitOnDrop {
        fn drop(&mut self) {
            self.dispatcher.emit(message(self.byte));
        }
    }

    /// Runs `work` on its own thread and fails if it has not returned within five seconds, which
    /// a dispatcher deadlocked on its own lock never does.
    fn returns_in_time(work: impl FnOnce() + Send + 'static) {
        let (done, returned) = std::sync::mpsc::channel();
        thread::spawn(move || {
            work();
            let _ = done.send(());
        });
        assert!(
            returned
                .recv_timeout(std::time::Duration::from_secs(5))
                .is_ok(),
            "the dispatcher deadlocked"
        );
    }

    #[test]
    fn a_replaced_handler_is_dropped_after_the_lock_is_released() {
        let dispatcher = Arc::new(Dispatcher::new());
        let (seen, recorder_handler) = recorder();
        let replacing = Arc::clone(&dispatcher);
        returns_in_time(move || {
            let probe = EmitOnDrop {
                dispatcher: Arc::clone(&replacing),
                byte: 9,
            };
            replacing.set_handler(Arc::new(move |_| {
                let _ = &probe;
            }));
            replacing.set_handler(recorder_handler);
        });
        assert_eq!(*seen.lock(), [message(9)]);
    }

    #[test]
    fn a_handler_that_replaces_itself_is_dropped_after_the_lock_is_released() {
        let dispatcher = Arc::new(Dispatcher::new());
        let (seen, recorder_handler) = recorder();
        let delivering = Arc::clone(&dispatcher);
        returns_in_time(move || {
            let probe = EmitOnDrop {
                dispatcher: Arc::clone(&delivering),
                byte: 9,
            };
            let next = Mutex::new(Some(recorder_handler));
            let replacer = Arc::clone(&delivering);
            delivering.set_handler(Arc::new(move |_| {
                let _ = &probe;
                if let Some(next) = next.lock().take() {
                    replacer.set_handler(next);
                }
            }));
            delivering.emit(message(1));
            delivering.emit(message(2));
        });
        assert_eq!(*seen.lock(), [message(9), message(2)]);
    }

    /// Panics when dropped, as a handler's captured state may.
    struct PanicOnDrop;

    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            panic!("the captured state panicked while dropped");
        }
    }

    #[test]
    fn a_handler_whose_captures_panic_when_dropped_leaves_delivery_working() {
        let dispatcher = Arc::new(Dispatcher::new());
        let (seen, recorder_handler) = recorder();
        let next = Mutex::new(Some(recorder_handler));
        let replacer = Arc::clone(&dispatcher);
        let probe = PanicOnDrop;
        dispatcher.set_handler(Arc::new(move |_| {
            let _ = &probe;
            if let Some(next) = next.lock().take() {
                replacer.set_handler(next);
            }
        }));
        let panicked = catch_unwind(AssertUnwindSafe(|| dispatcher.emit(message(1))));
        assert!(
            panicked.is_err(),
            "the destructor's panic reaches the caller"
        );
        dispatcher.emit(message(2));
        assert_eq!(*seen.lock(), [message(2)]);
    }
}
