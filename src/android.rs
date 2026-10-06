//! The FCM backend, driven by the Kotlin module in `android/`.
//!
//! `PushupsProvider` loads the app's library at process start and calls `Native.init`, which
//! hands Rust the `JavaVM` and the module's `Pushups` class. Kotlin reports every push and
//! every UI session change through the other `Native` methods, and Rust routes each event as
//! the plan's Android delivery table says.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicI64, Ordering};

use futures_channel::oneshot;
use jni::errors::ThrowRuntimeExAndDefault;
use jni::objects::{Global, JByteArray, JClass, JObject, JString, JValue};
use jni::sys::{jboolean, jlong};
use jni::{EnvUnowned, JavaVM, jni_sig, jni_str};
use parking_lot::Mutex;

use crate::dispatch::DISPATCHER;
use crate::queue::{Entry, QueueFile};
use crate::session::{Route, Session};
use crate::{Config, Error, Event, Message, Permission, Token};

/// What `Native.init` hands over, once per process.
struct Runtime {
    vm: JavaVM,
    pushups: Global<JClass<'static>>,
    state: Mutex<State>,
}

/// The routing state and the queue it writes to, behind one lock so a drain and a concurrent
/// push keep their order.
struct State {
    session: Session,
    queue: QueueFile,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Permission requests waiting for `Native.onPermissionResult`, by request id.
static PERMISSIONS: Mutex<BTreeMap<jlong, oneshot::Sender<bool>>> = Mutex::new(BTreeMap::new());
static NEXT_PERMISSION: AtomicI64 = AtomicI64::new(0);

const QUEUE_FILE: &str = "pushups-queue";

fn runtime() -> Result<&'static Runtime, Error> {
    RUNTIME.get().ok_or(Error::AndroidModuleMissing)
}

pub(crate) fn install(_config: Config) -> Result<(), Error> {
    runtime().map(|_| ())
}

pub(crate) fn register() -> Result<(), Error> {
    let runtime = runtime()?;
    runtime
        .vm
        .attach_current_thread(|env| -> jni::errors::Result<()> {
            env.call_static_method(&runtime.pushups, jni_str!("register"), jni_sig!("()V"), &[])?
                .v()
        })
        .map_err(|error| Error::Platform(error.to_string()))
}

pub(crate) async fn request_permission() -> Result<Permission, Error> {
    let runtime = runtime()?;
    let id = NEXT_PERMISSION.fetch_add(1, Ordering::Relaxed);
    let (sender, reply) = oneshot::channel();
    PERMISSIONS.lock().insert(id, sender);
    let asked = runtime
        .vm
        .attach_current_thread(|env| -> jni::errors::Result<()> {
            env.call_static_method(
                &runtime.pushups,
                jni_str!("requestPermission"),
                jni_sig!("(J)V"),
                &[JValue::Long(id)],
            )?
            .v()
        });
    if let Err(error) = asked {
        PERMISSIONS.lock().remove(&id);
        return Err(Error::Platform(error.to_string()));
    }
    match reply.await {
        Ok(true) => Ok(Permission::Granted),
        Ok(false) => Ok(Permission::Denied),
        Err(oneshot::Canceled) => Err(Error::Platform(
            "the permission request ended without an answer".to_owned(),
        )),
    }
}

/// Binds the UI session to the handler just set, and hands it the queue if the session was
/// waiting for one.
pub(crate) fn handler_set() {
    let Some(runtime) = RUNTIME.get() else {
        return;
    };
    {
        let mut state = runtime.state.lock();
        let (bound, drain) = state.session.on_handler_set();
        if drain {
            // A queue that cannot be read stays on disk and the session stays unbound, so
            // the next handler tries again.
            if let Ok(entries) = state.queue.take_all() {
                state.session = bound;
                for entry in entries {
                    DISPATCHER.enqueue(entry.into_event());
                }
            }
        } else {
            state.session = bound;
        }
    }
    DISPATCHER.flush();
}

/// Persists or emits an event from the platform, by the session's route.
fn receive(entry: Entry) {
    let Some(runtime) = RUNTIME.get() else {
        DISPATCHER.emit(entry.into_event());
        return;
    };
    {
        let state = runtime.state.lock();
        match state.session.route() {
            Route::Emit => DISPATCHER.enqueue(entry.into_event()),
            // A queue that cannot be written still leaves the event in memory, where a
            // handler of this process gets it.
            Route::Persist => {
                if state.queue.append(&entry).is_err() {
                    DISPATCHER.enqueue(entry.into_event());
                }
            }
        }
    }
    DISPATCHER.flush();
}

/// `Native.init(context, filesDir)`, from `PushupsProvider.onCreate`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_rs_pushups_Native_init<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    _context: JObject<'caller>,
    files_dir: JString<'caller>,
) {
    unowned
        .with_env(|env| -> jni::errors::Result<()> {
            let files_dir = PathBuf::from(files_dir.try_to_string(env)?);
            // Looked up inside a call from Kotlin, so the app's class loader finds it.
            let pushups = env.find_class(jni_str!("rs/pushups/Pushups"))?;
            let pushups = env.new_global_ref(pushups)?;
            let vm = env.get_java_vm()?;
            // The provider runs once per process, and a second call has nothing new to give.
            let _ = RUNTIME.set(Runtime {
                vm,
                pushups,
                state: Mutex::new(State {
                    session: Session::default(),
                    queue: QueueFile::new(files_dir.join(QUEUE_FILE)),
                }),
            });
            Ok(())
        })
        .resolve::<ThrowRuntimeExAndDefault>();
}

/// `Native.onSession(active)`, when a UI session begins or ends.
#[unsafe(no_mangle)]
pub extern "system" fn Java_rs_pushups_Native_onSession<'caller>(
    _unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    active: jboolean,
) {
    if let Some(runtime) = RUNTIME.get() {
        let mut state = runtime.state.lock();
        state.session = state.session.on_session(active);
    }
}

/// `Native.onToken(token)`, from `onNewToken`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_rs_pushups_Native_onToken<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    token: JString<'caller>,
) {
    unowned
        .with_env(|env| -> jni::errors::Result<()> {
            receive(Entry::FcmToken(token.try_to_string(env)?));
            Ok(())
        })
        .resolve::<ThrowRuntimeExAndDefault>();
}

/// `Native.onMessage(payload, startedApp)`, from `onMessageReceived` and notification taps.
#[unsafe(no_mangle)]
pub extern "system" fn Java_rs_pushups_Native_onMessage<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    payload: JByteArray<'caller>,
    started_app: jboolean,
) {
    unowned
        .with_env(|env| -> jni::errors::Result<()> {
            let payload = env.convert_byte_array(&payload)?;
            receive(Entry::Message(Message {
                payload,
                started_app,
            }));
            Ok(())
        })
        .resolve::<ThrowRuntimeExAndDefault>();
}

/// `Native.onMessagesDropped()`, from `onDeletedMessages`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_rs_pushups_Native_onMessagesDropped<'caller>(
    _unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
) {
    receive(Entry::MessagesDropped);
}

/// `Native.onRegistered(token, error)`, the answer to [`register`]. Exactly one is non-null.
#[unsafe(no_mangle)]
pub extern "system" fn Java_rs_pushups_Native_onRegistered<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    token: JString<'caller>,
    error: JString<'caller>,
) {
    unowned
        .with_env(|env| -> jni::errors::Result<()> {
            let event = if token.is_null() {
                let reason = if error.is_null() {
                    "FCM gave neither a token nor an error".to_owned()
                } else {
                    error.try_to_string(env)?
                };
                Event::RegistrationFailed(Error::Platform(reason))
            } else {
                Event::Token(Token::Fcm(token.try_to_string(env)?))
            };
            DISPATCHER.emit(event);
            Ok(())
        })
        .resolve::<ThrowRuntimeExAndDefault>();
}

/// `Native.onPermissionResult(requestId, granted)`, the answer to [`request_permission`].
#[unsafe(no_mangle)]
pub extern "system" fn Java_rs_pushups_Native_onPermissionResult<'caller>(
    _unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    request_id: jlong,
    granted: jboolean,
) {
    if let Some(sender) = PERMISSIONS.lock().remove(&request_id) {
        // A dropped receiver means the app stopped waiting for the answer.
        let _ = sender.send(granted);
    }
}
