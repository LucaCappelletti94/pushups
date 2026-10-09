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
    /// A session that begins after a handler was set binds to it and drains the queue into it.
    #[cfg(target_os = "android")]
    pub(crate) fn on_session(&mut self, active: bool) {
        let (next, drain) = self.session.on_session(active);
        if drain {
            self.drain_into(next, Session::Unbound);
        } else {
            self.session = next;
        }
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
        let (next, drain) = self.session.on_handler_set();
        if drain {
            self.drain_into(next, self.session);
        } else {
            self.session = next;
        }
    }

    /// Enqueues the queue for the handler and moves to `bound`. A queue that cannot be read stays
    /// on disk and the session moves to `unread`, unbound, so the next handler tries again.
    fn drain_into(&mut self, bound: Session, unread: Session) {
        match self.queue.take_all() {
            Ok(entries) => {
                self.session = bound;
                for entry in entries {
                    DISPATCHER.enqueue(entry.into_event());
                }
            }
            Err(_) => self.session = unread,
        }
    }
}
