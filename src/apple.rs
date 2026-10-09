//! The APNs backend for iOS and macOS, routing every event as the plan's Apple delivery table says.
//!
//! `install` runs on the main thread before the toolkit's loop. It takes over the notification center's delegate at once, since Apple requires it before launch finishes, and the app delegate when `didFinishLaunching` posts, since a toolkit may create it only inside `UIApplicationMain`.

mod runtime;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use dispatch2::{DispatchQueue, DispatchTime};
use futures_channel::oneshot;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_foundation::{NSError, NSFileManager, NSSearchPathDirectory, NSSearchPathDomainMask};
use objc2_user_notifications::{UNAuthorizationOptions, UNUserNotificationCenter};
use parking_lot::Mutex;

use crate::backend::Backend;
use crate::delivered::Delivered;
use crate::dispatch::DISPATCHER;
use crate::inbox::Inbox;
use crate::queue::{Entry, QueueFile};
use crate::session::Session;
use crate::started::{Arrival, Started};
use crate::{Config, Error, Event, Message, Permission, Token};

/// The routing state and the files it writes to, behind one lock so a drain and a concurrent push keep their order.
struct State {
    /// One UI session for the whole process, bound once `set_handler` was called.
    inbox: Inbox,
    /// The activation state `started_app` is read from.
    started: Started,
    delivered: Delivered,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

/// How long a background handler may run before the system is told the push is handled, inside Apple's 30 seconds.
const BACKGROUND_BOUND: Duration = Duration::from_secs(25);

/// The symbol `#[pushups::background_handler]` exports on Apple.
const BACKGROUND_HANDLER: &std::ffi::CStr = c"__pushups_background_handler";

/// The function behind [`BACKGROUND_HANDLER`].
type BackgroundHandler = unsafe extern "C" fn(*const u8, usize, bool);

pub(crate) struct Apple;

impl Backend for Apple {
    fn install(_config: Config) -> Result<(), Error> {
        let mtm = MainThreadMarker::new()
            .ok_or_else(|| Error::Platform("install must run on the main thread".to_owned()))?;
        if STATE.get().is_some() {
            return Ok(());
        }
        let directory = data_directory()?;
        let state = State {
            inbox: Inbox::new(Session::Unbound, QueueFile::new(directory.join("queue"))),
            started: Started::default(),
            delivered: Delivered::new(directory.join("delivered")),
        };
        if STATE.set(Mutex::new(state)).is_err() {
            return Ok(());
        }
        runtime::take_over_notification_delegate(mtm);
        runtime::observe_launch(mtm);
        Ok(())
    }

    fn register() -> Result<(), Error> {
        if STATE.get().is_none() {
            return Err(Error::NotConfigured);
        }
        DispatchQueue::main().exec_async(|| {
            if let Some(mtm) = MainThreadMarker::new() {
                runtime::register_for_remote_notifications(mtm);
            }
        });
        Ok(())
    }

    async fn request_permission() -> Result<Permission, Error> {
        if STATE.get().is_none() {
            return Err(Error::NotConfigured);
        }
        let (sender, reply) = oneshot::channel();
        let sender = Mutex::new(Some(sender));
        let answered = block2::RcBlock::new(move |granted: Bool, error: *mut NSError| {
            // SAFETY: the system passes a valid `NSError` or null, for this call only.
            let error = unsafe { error.as_ref() };
            let answer = match error {
                // `UNErrorCodeNotificationsNotAllowed`, the user or the system refusing notifications to this app.
                Some(error)
                    if error.domain().to_string() == "UNErrorDomain" && error.code() == 1 =>
                {
                    Ok(Permission::Denied)
                }
                Some(error) => Err(Error::Platform(error.localizedDescription().to_string())),
                None if granted.as_bool() => Ok(Permission::Granted),
                None => Ok(Permission::Denied),
            };
            if let Some(sender) = sender.lock().take() {
                let _ = sender.send(answer);
            }
        });
        UNUserNotificationCenter::currentNotificationCenter()
            .requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert
                    | UNAuthorizationOptions::Badge
                    | UNAuthorizationOptions::Sound,
                &answered,
            );
        reply.await.unwrap_or_else(|oneshot::Canceled| {
            Err(Error::Platform(
                "the permission request ended without an answer".to_owned(),
            ))
        })
    }

    /// Binds the process to the handler just set, and hands it the queue.
    fn handler_set() {
        if let Some(state) = STATE.get() {
            state.lock().inbox.handler_set();
            DISPATCHER.flush();
        }
    }
}

/// The Application Support directory of this app, where the queue and the delivered set live.
fn data_directory() -> Result<PathBuf, Error> {
    let manager = NSFileManager::defaultManager();
    let urls = manager.URLsForDirectory_inDomains(
        NSSearchPathDirectory::ApplicationSupportDirectory,
        NSSearchPathDomainMask::UserDomainMask,
    );
    let base = urls
        .firstObject()
        .and_then(|url| url.path())
        .ok_or_else(|| Error::Platform("no Application Support directory".to_owned()))?;
    let bundle = objc2_foundation::NSBundle::mainBundle()
        .bundleIdentifier()
        .map_or_else(|| "pushups".to_owned(), |id| id.to_string());
    let directory = PathBuf::from(base.to_string()).join(bundle).join("pushups");
    std::fs::create_dir_all(&directory).map_err(|error| Error::Platform(error.to_string()))?;
    Ok(directory)
}

/// Routes one push or tap, and returns the message when it is a first delivery the background handler should also see.
fn receive(payload: Vec<u8>, arrival: Arrival) -> Option<Message> {
    let state = STATE.get()?;
    let message = {
        let mut state = state.lock();
        // A delivered set that cannot be read or written lets the push through, since a repeat beats a loss.
        if !state.delivered.first_delivery(&payload).unwrap_or(true) {
            return None;
        }
        let started_app = state.started.started_app(arrival);
        let message = Message {
            payload,
            started_app,
        };
        state.inbox.receive(Entry::Message(message.clone()));
        message
    };
    DISPATCHER.flush();
    (arrival == Arrival::Push).then_some(message)
}

/// The process finished launching, in the background when the system started it for a push.
fn launched(in_background: bool) {
    if let Some(state) = STATE.get() {
        state.lock().started.launched(in_background);
    }
}

/// The app became active for the first time or again.
fn became_active() {
    if let Some(state) = STATE.get() {
        state.lock().started.became_active();
    }
}

fn token(token: Vec<u8>) {
    DISPATCHER.emit(Event::Token(Token::Apns(token)));
}

fn registration_failed(error: String) {
    DISPATCHER.emit(Event::RegistrationFailed(Error::Platform(error)));
}

/// The app's background handler, if `#[pushups::background_handler]` exported one.
fn background_handler() -> Option<BackgroundHandler> {
    static HANDLER: OnceLock<Option<usize>> = OnceLock::new();
    let address = *HANDLER.get_or_init(|| {
        // SAFETY: `RTLD_DEFAULT` searches every loaded image, and the name is NUL-terminated.
        let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, BACKGROUND_HANDLER.as_ptr()) };
        (!symbol.is_null()).then_some(symbol as usize)
    });
    // SAFETY: the macro exports this symbol with exactly the `BackgroundHandler` signature.
    address.map(|address| unsafe { std::mem::transmute::<usize, BackgroundHandler>(address) })
}

/// Runs the background handler for `message` off the main thread, then calls `done` on the main thread.
fn run_background_handler(message: Message, done: impl FnOnce() + Send + 'static) {
    let Some(handler) = background_handler() else {
        DispatchQueue::main().exec_async(done);
        return;
    };
    // A thread that cannot start leaves the push queued or delivered, and the bound completes it.
    let _ = std::thread::Builder::new()
        .name("pushups background handler".to_owned())
        .spawn(move || {
            // SAFETY: the pointer and length describe `message.payload`, alive for the call.
            unsafe {
                handler(
                    message.payload.as_ptr(),
                    message.payload.len(),
                    message.started_app,
                );
            }
            DispatchQueue::main().exec_async(done);
        });
}

thread_local! {
    /// Completion handlers waiting for a background handler or its bound, by id. Only the main thread touches it.
    static PENDING: RefCell<BTreeMap<u64, Box<dyn FnOnce()>>> = const { RefCell::new(BTreeMap::new()) };
    static NEXT_PENDING: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Calls `complete` once, on the main thread, when the background handler for `message` returns or [`BACKGROUND_BOUND`] passes.
fn finish_after_background_work(
    _mtm: MainThreadMarker,
    message: Option<Message>,
    complete: Box<dyn FnOnce()>,
) {
    let Some(message) = message else {
        complete();
        return;
    };
    let id = NEXT_PENDING.with(|next| {
        let id = next.get();
        next.set(id + 1);
        id
    });
    PENDING.with_borrow_mut(|pending| pending.insert(id, complete));
    let finish = move || {
        if let Some(complete) = PENDING.with_borrow_mut(|pending| pending.remove(&id)) {
            complete();
        }
    };
    if let Ok(when) = DispatchTime::try_from(BACKGROUND_BOUND) {
        let _ = DispatchQueue::main().after(when, finish);
    }
    run_background_handler(message, finish);
}

/// `Retained` objects the crate keeps for the life of the process, such as its own delegates, which the system holds weakly.
fn keep<T: ?Sized + objc2::Message>(object: Retained<T>) {
    std::mem::forget(object);
}
