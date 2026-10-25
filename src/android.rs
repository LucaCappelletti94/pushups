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
use jni::objects::{GlobalRef, JByteArray, JClass, JObject, JString, JValue};
use jni::sys::{JNI_FALSE, JNI_TRUE, jboolean, jlong};
use jni::{JNIEnv, JavaVM};
use parking_lot::Mutex;

use crate::android_boundary::with_attached;
use crate::backend::Backend;
use crate::dispatch::DISPATCHER;
use crate::inbox::{self, Inbox};
use crate::queue::Entry;
use crate::{Config, Error, Event, Message, Notification, Permission, Token};

/// What `Native.init` hands over, once per process.
struct Runtime {
    vm: JavaVM,
    pushups: GlobalRef,
    /// Behind one lock, so a drain and a concurrent push keep their order.
    inbox: Mutex<Inbox>,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// The app server's VAPID public key, base64url, when `install` got a `UnifiedPushConfig`.
static VAPID: OnceLock<String> = OnceLock::new();

/// Permission requests waiting for `Native.onPermissionResult`, by request id.
static PERMISSIONS: Mutex<BTreeMap<jlong, oneshot::Sender<bool>>> = Mutex::new(BTreeMap::new());
static NEXT_PERMISSION: AtomicI64 = AtomicI64::new(0);

/// The status codes the module's `permissionStatus` returns, mirroring its constants.
const STATUS_GRANTED: i32 = 0;
const STATUS_DENIED: i32 = 1;
const STATUS_NOT_DETERMINED: i32 = 2;

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
            let _ = VAPID.set(unified_push.vapid_public_key.as_base64url());
        }
        runtime().map(|_| ())
    }

    fn register() -> Result<(), Error> {
        let runtime = runtime()?;
        with_attached(&runtime.vm, |env| -> jni::errors::Result<()> {
            let vapid = match VAPID.get() {
                Some(vapid) => JObject::from(env.new_string(vapid)?),
                None => JObject::null(),
            };
            env.call_static_method(
                &runtime.pushups,
                "register",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&vapid)],
            )?
            .v()
        })
        .map_err(Error::platform)
    }

    async fn request_permission() -> Result<Permission, Error> {
        let runtime = runtime()?;
        let id = NEXT_PERMISSION.fetch_add(1, Ordering::Relaxed);
        let (sender, reply) = oneshot::channel();
        PERMISSIONS.lock().insert(id, sender);
        let asked = with_attached(&runtime.vm, |env| -> jni::errors::Result<()> {
            env.call_static_method(
                &runtime.pushups,
                "requestPermission",
                "(J)V",
                &[JValue::Long(id)],
            )?
            .v()
        });
        if let Err(error) = asked {
            PERMISSIONS.lock().remove(&id);
            return Err(Error::platform(error));
        }
        match reply.await {
            Ok(true) => Ok(Permission::Granted),
            Ok(false) => Ok(Permission::Denied),
            Err(oneshot::Canceled) => Err(Error::permission_unanswered()),
        }
    }

    fn permission_status() -> impl Future<Output = Result<Permission, Error>> {
        let result = runtime().and_then(|runtime| {
            let status = with_attached(&runtime.vm, |env| -> jni::errors::Result<i32> {
                env.call_static_method(&runtime.pushups, "permissionStatus", "()I", &[])?
                    .i()
            });
            status.map_err(Error::platform).and_then(permission_from)
        });
        std::future::ready(result)
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

/// Shows `notification` through the module's presentation, without opening an Activity.
pub(crate) fn show(notification: &Notification) -> Result<(), Error> {
    let runtime = runtime()?;
    let refused = with_attached(&runtime.vm, |env| -> jni::errors::Result<Option<String>> {
        let title = JObject::from(env.new_string(&notification.title)?);
        let body = string(env, notification.body.as_deref())?;
        let navigate = string(env, notification.navigate.as_deref())?;
        let tag = string(env, notification.tag.as_deref())?;
        let icon = string(env, notification.icon.as_deref())?;
        let data = string(env, notification.data.as_deref())?;
        let message = env
            .call_static_method(
                &runtime.pushups,
                "present",
                "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
                &[
                    JValue::Object(&title),
                    JValue::Object(&body),
                    JValue::Object(&navigate),
                    JValue::Object(&tag),
                    JValue::Object(&icon),
                    JValue::Object(&data),
                ],
            )?
            .l()?;
        if message.as_raw().is_null() {
            Ok(None)
        } else {
            Ok(Some(env.get_string(&JString::from(message))?.into()))
        }
    })
    .map_err(Error::platform)?;
    refused.map_or(Ok(()), |message| Err(Error::platform(message)))
}

/// A Java string for `value`, or `null`.
fn string<'local>(
    env: &mut JNIEnv<'local>,
    value: Option<&str>,
) -> jni::errors::Result<JObject<'local>> {
    match value {
        Some(value) => Ok(JObject::from(env.new_string(value)?)),
        None => Ok(JObject::null()),
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

crate::__pushups_android_exports! {
    /// `Native.handshake(moduleVersion)`, from `PushupsProvider.onCreate` before any other export.
    /// Whether the Kotlin module is this crate's version. Its signature never changes, so a module
    /// and a crate of any two versions can make this call.
    fn Java_rs_pushups_Native_handshake<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
        module_version: JString<'local>,
    ) -> jboolean {
        let module: String = env.get_string(&module_version)?.into();
        if module == env!("CARGO_PKG_VERSION") {
            Ok(JNI_TRUE)
        } else {
            let _ = MISMATCHED_MODULE.set(module);
            Ok(JNI_FALSE)
        }
    }

    /// `Native.init(context, filesDir)`, from `PushupsProvider.onCreate`.
    fn Java_rs_pushups_Native_init<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
        _context: JObject<'local>,
        files_dir: JString<'local>,
    ) -> () {
        let files_dir: String = env.get_string(&files_dir)?.into();
        let files_dir = PathBuf::from(files_dir);
        // Looked up inside a call from Kotlin, so the app's class loader finds it.
        let pushups = env.find_class("rs/pushups/Pushups")?;
        let pushups = env.new_global_ref(&pushups)?;
        let vm = env.get_java_vm()?;
        // The provider runs once per process, and a second call has nothing new to give.
        let _ = RUNTIME.set(Runtime {
            vm,
            pushups,
            inbox: Mutex::new(Inbox::headless(files_dir.join(QUEUE_FILE))),
        });
        Ok(())
    }

    /// `Native.onSession(active)`, when a UI session begins or ends.
    fn Java_rs_pushups_Native_onSession<'local>(
        _env: JNIEnv<'local>,
        _class: JClass<'local>,
        active: jboolean,
    ) -> () {
        if let Some(runtime) = RUNTIME.get() {
            inbox::session_changed(&runtime.inbox, active != JNI_FALSE);
        }
        Ok(())
    }

    /// `Native.onToken(token)`, from `onNewToken`.
    fn Java_rs_pushups_Native_onToken<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
        token: JString<'local>,
    ) -> () {
        let token: String = env.get_string(&token)?.into();
        receive(Entry::FcmToken(token));
        Ok(())
    }

    /// `Native.onMessage(payload, startedApp)`, from `onMessageReceived` and notification taps.
    fn Java_rs_pushups_Native_onMessage<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
        payload: JByteArray<'local>,
        started_app: jboolean,
    ) -> () {
        let payload = env.convert_byte_array(&payload)?;
        receive(Entry::Message(Message {
            payload,
            started_app: started_app != JNI_FALSE,
        }));
        Ok(())
    }

    /// `Native.onMessagesDropped()`, from `onDeletedMessages`.
    fn Java_rs_pushups_Native_onMessagesDropped<'local>(
        _env: JNIEnv<'local>,
        _class: JClass<'local>,
    ) -> () {
        receive(Entry::MessagesDropped);
        Ok(())
    }

    /// `Native.onWebPushToken(endpoint, p256dh, auth)`, from a UnifiedPush endpoint.
    fn Java_rs_pushups_Native_onWebPushToken<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
        endpoint: JString<'local>,
        p256dh: JByteArray<'local>,
        auth: JByteArray<'local>,
    ) -> () {
        let endpoint: String = env.get_string(&endpoint)?.into();
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
    }

    /// `Native.onUnregistered()`, when the UnifiedPush distributor dropped the registration.
    fn Java_rs_pushups_Native_onUnregistered<'local>(
        _env: JNIEnv<'local>,
        _class: JClass<'local>,
    ) -> () {
        receive(Entry::Unregistered);
        Ok(())
    }

    /// `Native.onRegistered(token, error)`, the answer to [`register`]. Exactly one is non-null.
    fn Java_rs_pushups_Native_onRegistered<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
        token: JString<'local>,
        error: JString<'local>,
    ) -> () {
        let event = if token.as_raw().is_null() {
            let reason = if error.as_raw().is_null() {
                "FCM gave neither a token nor an error".to_owned()
            } else {
                env.get_string(&error)?.into()
            };
            Event::registration_failed(reason)
        } else {
            let token: String = env.get_string(&token)?.into();
            Event::Token(Token::Fcm { token, keys: None })
        };
        DISPATCHER.emit(event);
        Ok(())
    }

    /// `Native.onPermissionResult(requestId, granted)`, the answer to [`request_permission`].
    fn Java_rs_pushups_Native_onPermissionResult<'local>(
        _env: JNIEnv<'local>,
        _class: JClass<'local>,
        request_id: jlong,
        granted: jboolean,
    ) -> () {
        if let Some(sender) = PERMISSIONS.lock().remove(&request_id) {
            // A dropped receiver means the app stopped waiting for the answer.
            let _ = sender.send(granted != JNI_FALSE);
        }
        Ok(())
    }
}

/// Maps the status the module's `permissionStatus` returns to the permission the app sees, and
/// refuses a value the module does not report.
fn permission_from(status: i32) -> Result<Permission, Error> {
    match status {
        STATUS_GRANTED => Ok(Permission::Granted),
        STATUS_DENIED => Ok(Permission::Denied),
        STATUS_NOT_DETERMINED => Ok(Permission::NotDetermined),
        other => Err(Error::Platform(format!(
            "the module reported the unknown permission status {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_module_statuses_map_to_the_permissions() {
        assert_eq!(permission_from(STATUS_GRANTED), Ok(Permission::Granted));
        assert_eq!(permission_from(STATUS_DENIED), Ok(Permission::Denied));
        assert_eq!(
            permission_from(STATUS_NOT_DETERMINED),
            Ok(Permission::NotDetermined)
        );
    }

    #[test]
    fn an_unknown_module_status_is_a_platform_error() {
        for other in [i32::MIN, -1, 3, i32::MAX] {
            assert!(permission_from(other).is_err());
        }
    }
}
