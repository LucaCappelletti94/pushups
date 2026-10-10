//! The FCM and UnifiedPush backend, driven by the Kotlin module in `android/`.
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

use crate::backend::Backend;
use crate::dispatch::DISPATCHER;
use crate::inbox::Inbox;
use crate::queue::{Entry, QueueFile};
use crate::session::Session;
use crate::token::base64url;
use crate::{Config, Error, Event, Message, Permission, Token};

/// What `Native.init` hands over, once per process.
struct Runtime {
    vm: JavaVM,
    pushups: Global<JClass<'static>>,
    /// Behind one lock, so a drain and a concurrent push keep their order.
    inbox: Mutex<Inbox>,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// The app server's VAPID public key, base64url, when `install` got a `UnifiedPushConfig`.
static VAPID: OnceLock<String> = OnceLock::new();

/// Permission requests waiting for `Native.onPermissionResult`, by request id.
static PERMISSIONS: Mutex<BTreeMap<jlong, oneshot::Sender<bool>>> = Mutex::new(BTreeMap::new());
static NEXT_PERMISSION: AtomicI64 = AtomicI64::new(0);

/// The module version the handshake saw, when it differed from the crate's.
static MISMATCHED_MODULE: OnceLock<String> = OnceLock::new();

const QUEUE_FILE: &str = "pushups-queue";

fn runtime() -> Result<&'static Runtime, Error> {
    RUNTIME.get().ok_or_else(|| match MISMATCHED_MODULE.get() {
        Some(module) => Error::AndroidModuleMismatch {
            module: module.clone(),
            crate_version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        None => Error::AndroidModuleMissing,
    })
}

pub(crate) struct Android;

impl Backend for Android {
    fn install(config: Config) -> Result<(), Error> {
        if let Some(unified_push) = config.unified_push {
            // A second `install` keeps the first key, as it keeps the first runtime.
            let _ = VAPID.set(base64url(&unified_push.vapid_public_key));
        }
        runtime().map(|_| ())
    }

    fn register() -> Result<(), Error> {
        let runtime = runtime()?;
        runtime
            .vm
            .attach_current_thread(|env| -> jni::errors::Result<()> {
                let vapid = match VAPID.get() {
                    Some(vapid) => JObject::from(env.new_string(vapid)?),
                    None => JObject::null(),
                };
                env.call_static_method(
                    &runtime.pushups,
                    jni_str!("register"),
                    jni_sig!("(Ljava/lang/String;)V"),
                    &[JValue::Object(&vapid)],
                )?
                .v()
            })
            .map_err(|error| Error::Platform(error.to_string()))
    }

    async fn request_permission() -> Result<Permission, Error> {
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
    fn handler_set() {
        if let Some(runtime) = RUNTIME.get() {
            runtime.inbox.lock().handler_set();
            DISPATCHER.flush();
        }
    }
}

/// Persists or emits an event from the platform, by the session's route.
fn receive(entry: Entry) {
    match RUNTIME.get() {
        Some(runtime) => runtime.inbox.lock().receive(entry),
        None => DISPATCHER.enqueue(entry.into_event()),
    }
    DISPATCHER.flush();
}

/// `Native.handshake(moduleVersion)`, from `PushupsProvider.onCreate` before any other export.
/// Whether the Kotlin module is this crate's version. Its signature never changes, so a module
/// and a crate of any two versions can make this call.
#[unsafe(no_mangle)]
pub extern "system" fn Java_rs_pushups_Native_handshake<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    module_version: JString<'caller>,
) -> jboolean {
    let module = unowned
        .with_env(|env| -> jni::errors::Result<String> { module_version.try_to_string(env) })
        .resolve::<ThrowRuntimeExAndDefault>();
    if module == env!("CARGO_PKG_VERSION") {
        return true;
    }
    let _ = MISMATCHED_MODULE.set(module);
    false
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
                inbox: Mutex::new(Inbox::new(
                    Session::default(),
                    QueueFile::new(files_dir.join(QUEUE_FILE)),
                )),
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
        runtime.inbox.lock().on_session(active);
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

/// `Native.onWebPushToken(endpoint, p256dh, auth)`, from a UnifiedPush endpoint.
#[unsafe(no_mangle)]
pub extern "system" fn Java_rs_pushups_Native_onWebPushToken<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    endpoint: JString<'caller>,
    p256dh: JByteArray<'caller>,
    auth: JByteArray<'caller>,
) {
    unowned
        .with_env(|env| -> jni::errors::Result<()> {
            let endpoint = endpoint.try_to_string(env)?;
            let p256dh = env.convert_byte_array(&p256dh)?;
            let auth = env.convert_byte_array(&auth)?;
            let entry = match (<[u8; 65]>::try_from(p256dh), <[u8; 16]>::try_from(auth)) {
                (Ok(p256dh), Ok(auth)) => Entry::WebPushToken {
                    endpoint,
                    p256dh,
                    auth,
                },
                _ => Entry::RegistrationFailed(
                    "the distributor's endpoint has no valid Web Push keys".to_owned(),
                ),
            };
            receive(entry);
            Ok(())
        })
        .resolve::<ThrowRuntimeExAndDefault>();
}

/// `Native.onUnregistered()`, when the UnifiedPush distributor dropped the registration.
#[unsafe(no_mangle)]
pub extern "system" fn Java_rs_pushups_Native_onUnregistered<'caller>(
    _unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
) {
    receive(Entry::RegistrationFailed(
        "the distributor unregistered the app".to_owned(),
    ));
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
