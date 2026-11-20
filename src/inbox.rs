//! Where an event goes on a backend with a queue file, by the state of the app's UI, and the
//! drain into the first handler of a UI session.

use std::path::PathBuf;

use crate::dispatch::DISPATCHER;
use crate::queue::{Entry, QueueFile};
use crate::session::{Route, Session};
#[cfg(any(target_os = "android", test))]
use parking_lot::Mutex;

/// The session and the queue it writes to. Callers keep it under the lock that orders their
/// events and call `DISPATCHER.flush()` once the lock is released.
pub(crate) struct Inbox {
    session: Session,
    queue: QueueFile,
}

impl Inbox {
    /// The inbox of a process with no UI yet, such as one the system started for a push, whose
    /// events wait in the queue file at `queue` for a UI session's handler.
    #[cfg(any(target_os = "android", target_os = "linux", test))]
    pub(crate) fn headless(queue: PathBuf) -> Self {
        Self {
            session: Session::Headless,
            queue: QueueFile::new(queue),
        }
    }

    /// The inbox of a process whose UI is up, whose events wait in the queue file at `queue`
    /// until its handler is set.
    #[cfg(any(target_os = "ios", target_os = "macos", target_os = "linux"))]
    pub(crate) fn for_ui(queue: PathBuf) -> Self {
        Self {
            session: Session::Unbound,
            queue: QueueFile::new(queue),
        }
    }

    /// Makes this process the owner of the queue file, waiting for a previous owner such as the
    /// headless process this one replaced, so nothing that process persists comes after a drain.
    #[cfg(target_os = "linux")]
    pub(crate) fn claim_queue(&self) -> std::io::Result<()> {
        self.queue.claim()
    }

    /// A UI session began (`true`) or ended (`false`), which only Android reports while running.
    /// A session that begins after a handler was set binds to it and drains the queue into it,
    /// and a handler kept from the session before binds only while the thread that set it runs.
    #[cfg(any(target_os = "android", test))]
    pub(crate) fn on_session(&mut self, active: bool) {
        let (next, drain) = self
            .session
            .on_session(active, DISPATCHER.handler_thread_alive());
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

/// A UI session began (`true`) or ended (`false`) on the inbox behind `inbox`, then delivers what
/// the session drained once the lock is released.
#[cfg(any(target_os = "android", test))]
pub(crate) fn session_changed(inbox: &Mutex<Inbox>, active: bool) {
    inbox.lock().on_session(active);
    DISPATCHER.flush();
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::Event;
    use crate::dispatch::DISPATCHER_TEST_LOCK;

    /// A headless process whose handler is set queues to the file, and the UI session that then
    /// begins hands the queue to that handler with no later event needed to deliver it.
    #[test]
    fn a_session_beginning_after_a_headless_handler_delivers_the_queue() {
        let _guard = DISPATCHER_TEST_LOCK.lock();
        let dir = tempfile::tempdir().unwrap();
        let inbox = Mutex::new(Inbox::headless(dir.path().join("queue")));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        DISPATCHER.set_handler(Arc::new(move |event| sink.lock().push(event)));
        inbox.lock().handler_set();
        inbox.lock().receive(Entry::FcmToken {
            token: "queued".to_owned(),
            keys: None,
        });
        assert!(
            seen.lock().is_empty(),
            "a headless process persists its events"
        );
        session_changed(&inbox, true);
        assert_eq!(
            *seen.lock(),
            vec![Event::Token(crate::Token::Fcm {
                token: "queued".to_owned(),
                keys: None
            })]
        );
    }

    /// A bound session hands new events to its handler at once, through the dispatcher.
    #[test]
    fn a_bound_session_emits_new_events_to_its_handler() {
        let _guard = DISPATCHER_TEST_LOCK.lock();
        let dir = tempfile::tempdir().unwrap();
        let inbox = Mutex::new(Inbox::headless(dir.path().join("queue")));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        DISPATCHER.set_handler(Arc::new(move |event| sink.lock().push(event)));
        inbox.lock().handler_set();
        session_changed(&inbox, true);
        inbox.lock().receive(Entry::FcmToken {
            token: "live".to_owned(),
            keys: None,
        });
        DISPATCHER.flush();
        assert_eq!(
            *seen.lock(),
            vec![Event::Token(crate::Token::Fcm {
                token: "live".to_owned(),
                keys: None
            })]
        );
    }

    /// An event whose queue cannot be written still reaches the handler of this process.
    #[test]
    fn an_event_whose_queue_cannot_be_written_reaches_the_handler() {
        let _guard = DISPATCHER_TEST_LOCK.lock();
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("queue")).unwrap();
        let inbox = Mutex::new(Inbox::headless(dir.path().join("queue")));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        DISPATCHER.set_handler(Arc::new(move |event| sink.lock().push(event)));
        inbox.lock().handler_set();
        inbox.lock().receive(Entry::FcmToken {
            token: "live".to_owned(),
            keys: None,
        });
        DISPATCHER.flush();
        assert_eq!(
            *seen.lock(),
            vec![Event::Token(crate::Token::Fcm {
                token: "live".to_owned(),
                keys: None
            })]
        );
    }

    /// A queue that cannot be read at a drain is not lost. The session goes unread, the entry
    /// stays on disk, and the handler set next drains it.
    #[test]
    fn a_queue_that_cannot_be_read_stays_for_the_next_handler() {
        let _guard = DISPATCHER_TEST_LOCK.lock();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue");
        let kept = dir.path().join("queue.kept");
        let inbox = Mutex::new(Inbox::headless(path.clone()));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        DISPATCHER.set_handler(Arc::new(move |event| sink.lock().push(event)));
        inbox.lock().handler_set();
        inbox.lock().receive(Entry::FcmToken {
            token: "queued".to_owned(),
            keys: None,
        });
        assert!(
            seen.lock().is_empty(),
            "a headless process persists its events"
        );
        // The queue's path becomes a directory, so the drain cannot read it.
        std::fs::rename(&path, &kept).unwrap();
        std::fs::create_dir(&path).unwrap();
        session_changed(&inbox, true);
        assert!(seen.lock().is_empty(), "an unreadable queue is not drained");
        std::fs::remove_dir(&path).unwrap();
        std::fs::rename(&kept, &path).unwrap();
        inbox.lock().handler_set();
        DISPATCHER.flush();
        assert_eq!(
            *seen.lock(),
            vec![Event::Token(crate::Token::Fcm {
                token: "queued".to_owned(),
                keys: None
            })]
        );
    }
}
