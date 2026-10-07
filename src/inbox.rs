//! Where an event goes on a backend with a queue file, by the state of the app's UI, and the
//! drain into the first handler of a UI session.

use crate::dispatch::DISPATCHER;
use crate::queue::{Entry, QueueFile};
use crate::session::{Route, Session};

/// The session and the queue it writes to. Callers keep it under the lock that orders their
/// events and call `DISPATCHER.flush()` once the lock is released.
pub(crate) struct Inbox {
    session: Session,
    queue: QueueFile,
}

impl Inbox {
    pub(crate) fn new(session: Session, queue: QueueFile) -> Self {
        Self { session, queue }
    }

    /// A UI session began (`true`) or ended (`false`), which only Android reports while running.
    #[cfg(target_os = "android")]
    pub(crate) fn on_session(&mut self, active: bool) {
        self.session = self.session.on_session(active);
    }

    /// Persists or enqueues an event, by the session's route.
    pub(crate) fn receive(&self, entry: Entry) {
        match self.session.route() {
            Route::Emit => DISPATCHER.enqueue(entry.into_event()),
            // A queue that cannot be written still leaves the event in memory, where a handler of
            // this process gets it.
            Route::Persist => {
                if self.queue.append(&entry).is_err() {
                    DISPATCHER.enqueue(entry.into_event());
                }
            }
        }
    }

    /// Binds the session to the handler just set, and enqueues the queue if it was waiting for one.
    pub(crate) fn handler_set(&mut self) {
        let (bound, drain) = self.session.on_handler_set();
        if !drain {
            self.session = bound;
            return;
        }
        // A queue that cannot be read stays on disk and the session stays unbound, so the next
        // handler tries again.
        if let Ok(entries) = self.queue.take_all() {
            self.session = bound;
            for entry in entries {
                DISPATCHER.enqueue(entry.into_event());
            }
        }
    }
}
