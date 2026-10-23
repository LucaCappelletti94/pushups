//! The app library the Android instrumented tests load in place of a real app's cdylib.
//!
//! It holds the Firebase configuration and the background handler, as an app would, and exports
//! `rs.pushups.ci.Probe`, through which the Kotlin tests call `pushups`' public API and read
//! back every event as a line of text. It is built with `-Cinstrument-coverage`, and
//! `Probe.writeCoverage` writes the profile the CI job turns into LCOV.

use std::ffi::{CString, c_char, c_int};
use std::panic::panic_any;
use std::thread;

use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{JNI_FALSE, JNI_TRUE, jboolean, jstring};
use parking_lot::Mutex;
use pushups::{Config, Event, Message, Notification, Token, UnifiedPushConfig, VapidPublicKey};

#[cfg(feature = "config-real")]
pushups::firebase_config!("google-services.json");
#[cfg(feature = "config-fixture")]
pushups::firebase_config!("../../../tests/fixtures/google-services.json");
#[cfg(feature = "config-mismatched")]
pushups::firebase_config!("mismatched-services.json");

/// Every event the handler received, one line each, in order.
static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Every background push and fixture presentation failure, one line each.
static HANDLED: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// The answer to the last `startPermission`, until `permission` takes it.
static PERMISSION: Mutex<Option<String>> = Mutex::new(None);

unsafe extern "C" {
    fn __llvm_profile_set_filename(name: *const c_char);
    fn __llvm_profile_write_file() -> c_int;
}

#[cfg(feature = "background-handler")]
#[pushups::background_handler]
fn on_push(_context: pushups::Context, message: Message) {
    HANDLED.lock().push(describe_message(&message));
    if let Some(notification) = fixture_notification(&message.payload) {
        // Exercise `JavaVM` attachment without capturing the callback's `Context`.
        let failure = match std::thread::spawn(move || notification.show()).join() {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(format!("fixture presentation failed with {error}")),
            Err(_) => Some("fixture presentation thread panicked".to_owned()),
        };
        if let Some(failure) = failure {
            HANDLED.lock().push(failure);
        }
    }
}

/// The fixture a headless test push carries, as `{"headless":"<id>"}` with a unique `id`.
#[cfg(feature = "background-handler")]
fn fixture_notification(payload: &[u8]) -> Option<Notification> {
    const MARKER: &str = "{\"headless\":\"";
    let text = std::str::from_utf8(payload).ok()?;
    let id = text.strip_prefix(MARKER)?.strip_suffix("\"}")?;
    if id.is_empty() || id.contains('"') {
        return None;
    }
    Some(
        Notification::new(format!("pushups {id}"))
            .body("shown by the background handler")
            .tag(id)
            .data(text.to_owned()),
    )
}

fn describe(event: &Event) -> String {
    match event {
        Event::Token(Token::Fcm(token)) => format!("token {token}"),
        Event::Token(token) => match token.web_push_subscription() {
            Some(subscription) => format!(
                "webpush {} {} {}",
                subscription.endpoint, subscription.keys.p256dh, subscription.keys.auth
            ),
            None => format!("token {token:?}"),
        },
        Event::RegistrationFailed(error) => format!("failed {error}"),
        Event::Message(message) => describe_message(message),
        Event::MessagesDropped => "dropped".to_owned(),
    }
}

fn describe_message(message: &Message) -> String {
    format!(
        "message started_app={} {}",
        message.started_app,
        String::from_utf8_lossy(&message.payload)
    )
}

/// A Java string holding `text`, or `null` for `None`.
fn java_string<'local>(
    env: &mut JNIEnv<'local>,
    text: Option<String>,
) -> jni::errors::Result<jstring> {
    match text {
        Some(text) => env.new_string(text).map(JString::into_raw),
        None => Ok(JString::default().into_raw()),
    }
}

/// A `String` from a Java string, `None` for `null`.
fn optional_string<'local>(
    env: &mut JNIEnv<'local>,
    value: &JString<'local>,
) -> jni::errors::Result<Option<String>> {
    if value.as_raw().is_null() {
        Ok(None)
    } else {
        Ok(Some(env.get_string(value)?.into()))
    }
}

/// A panic payload whose destructor panics, for the `boundaryPanickingDrop` case.
struct PanickingPayload {
    id: String,
}

impl Drop for PanickingPayload {
    fn drop(&mut self) {
        panic!("jni boundary payload drop {id}", id = self.id);
    }
}

pushups::__private::android::exports! {
    /// `Probe.install()`: `null`, or the error of `pushups::install`.
    fn Java_rs_pushups_ci_Probe_install<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
    ) -> jstring {
        java_string(
            env,
            pushups::install(Config::new()).err().map(|error| error.to_string()),
        )
    }

    /// `Probe.installWithVapid(vapid)`: `null`, or the error of `pushups::install` with a
    /// UnifiedPush part for the VAPID public key in base64url, or of decoding that key.
    fn Java_rs_pushups_ci_Probe_installWithVapid<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
        vapid: JString<'local>,
    ) -> jstring {
        let vapid: String = env.get_string(&vapid)?.into();
        let error = VapidPublicKey::from_base64url(&vapid)
            .and_then(|key| {
                pushups::install(Config::new().unified_push(UnifiedPushConfig::new(key)))
            })
            .err()
            .map(|error| error.to_string());
        java_string(env, error)
    }

    /// `Probe.setHandler()`: records every event from now on.
    fn Java_rs_pushups_ci_Probe_setHandler<'local>(
        _env: JNIEnv<'local>,
        _class: JClass<'local>,
    ) -> () {
        pushups::set_handler(|event| EVENTS.lock().push(describe(&event)));
        Ok(())
    }

    /// `Probe.setPanickingHandler(kind, requestId)`: installs a handler that panics on the
    /// selected event, quoting `requestId` in the panic.
    fn Java_rs_pushups_ci_Probe_setPanickingHandler<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
        kind: JString<'local>,
        request_id: JString<'local>,
    ) -> () {
        let kind: String = env.get_string(&kind)?.into();
        let request_id: String = env.get_string(&request_id)?.into();
        pushups::set_handler(move |event| {
            let selected = match kind.as_str() {
                "dropped" => matches!(&event, Event::MessagesDropped),
                "unregistered" => matches!(
                    &event,
                    Event::RegistrationFailed(pushups::Error::Unregistered)
                ),
                "registered" => matches!(&event, Event::Token(Token::Fcm(_))),
                "session" => matches!(&event, Event::Message(_)),
                _ => false,
            };
            assert!(!selected, "JNI handler panic {request_id}");
            EVENTS.lock().push(describe(&event));
        });
        Ok(())
    }

    /// `Probe.register()`: `null`, or the error of `pushups::register`.
    fn Java_rs_pushups_ci_Probe_register<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
    ) -> jstring {
        java_string(
            env,
            pushups::register().err().map(|error| error.to_string()),
        )
    }

    /// `Probe.startPermission()`: asks for the permission on a thread of its own, since the
    /// test thread has to answer the prompt.
    fn Java_rs_pushups_ci_Probe_startPermission<'local>(
        _env: JNIEnv<'local>,
        _class: JClass<'local>,
    ) -> () {
        PERMISSION.lock().take();
        thread::spawn(|| {
            let answer = match futures_executor::block_on(pushups::request_permission()) {
                Ok(permission) => format!("{permission:?}"),
                Err(error) => format!("error {error}"),
            };
            *PERMISSION.lock() = Some(answer);
        });
        Ok(())
    }

    /// `Probe.permission()`: the answer to `startPermission`, or `null` while it is pending.
    fn Java_rs_pushups_ci_Probe_permission<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
    ) -> jstring {
        java_string(env, PERMISSION.lock().take())
    }

    /// `Probe.permissionStatus()`: the answer of `pushups::permission_status` as `Granted`,
    /// `Denied` or `NotDetermined`, or `error ...`.
    fn Java_rs_pushups_ci_Probe_permissionStatus<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
    ) -> jstring {
        let answer = match futures_executor::block_on(pushups::permission_status()) {
            Ok(permission) => format!("{permission:?}"),
            Err(error) => format!("error {error}"),
        };
        java_string(env, Some(answer))
    }

    /// `Probe.events()`: every event so far, one per line.
    fn Java_rs_pushups_ci_Probe_events<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
    ) -> jstring {
        java_string(env, Some(EVENTS.lock().join("\n")))
    }

    /// `Probe.handled()`: every push the background handler received, one per line.
    fn Java_rs_pushups_ci_Probe_handled<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
    ) -> jstring {
        java_string(env, Some(HANDLED.lock().join("\n")))
    }

    /// `Probe.presentBytes(...)`: `null`, or the error of `Notification::show`. `data` arrives as
    /// UTF-8 bytes, which JNI 0.21's string decoding would corrupt as a Java string.
    fn Java_rs_pushups_ci_Probe_presentBytes<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
        title: JString<'local>,
        body: JString<'local>,
        navigate: JString<'local>,
        tag: JString<'local>,
        icon: JString<'local>,
        data: JByteArray<'local>,
    ) -> jstring {
        let title: String = env.get_string(&title)?.into();
        let mut notification = Notification::new(title);
        if let Some(body) = optional_string(env, &body)? {
            notification = notification.body(body);
        }
        if let Some(navigate) = optional_string(env, &navigate)? {
            notification = notification.navigate(navigate);
        }
        if let Some(tag) = optional_string(env, &tag)? {
            notification = notification.tag(tag);
        }
        if let Some(icon) = optional_string(env, &icon)? {
            notification = notification.icon(icon);
        }
        if !data.as_raw().is_null() {
            match String::from_utf8(env.convert_byte_array(&data)?) {
                Ok(data) => notification = notification.data(data),
                Err(error) => return java_string(env, Some(format!("invalid UTF-8 data: {error}"))),
            }
        }
        java_string(env, notification.show().err().map(|error| error.to_string()))
    }

    /// `Probe.writeCoverage(path)`: writes the coverage profile to `path`, `true` on success.
    fn Java_rs_pushups_ci_Probe_writeCoverage<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
        path: JString<'local>,
    ) -> jboolean {
        let path: String = env.get_string(&path)?.into();
        let path = CString::new(path).unwrap_or_default();
        // SAFETY: `path` is a NUL-terminated string that outlives both calls, and the profiling
        // runtime copies the name before the write returns.
        let written = unsafe {
            __llvm_profile_set_filename(path.as_ptr());
            __llvm_profile_write_file()
        };
        Ok(if written == 0 { JNI_TRUE } else { JNI_FALSE })
    }

    /// `Probe.boundaryTrue()`: `JNI_TRUE`, the success boolean return.
    fn Java_rs_pushups_ci_Probe_boundaryTrue<'local>(
        _env: JNIEnv<'local>,
        _class: JClass<'local>,
    ) -> jboolean {
        Ok(JNI_TRUE)
    }

    /// `Probe.boundaryNull()`: `null`, the success null return.
    fn Java_rs_pushups_ci_Probe_boundaryNull<'local>(
        _env: JNIEnv<'local>,
        _class: JClass<'local>,
    ) -> jstring {
        Ok(JString::default().into_raw())
    }

    /// `Probe.boundaryObject(id)`: `id` through a fresh Java string, the success object return.
    fn Java_rs_pushups_ci_Probe_boundaryObject<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
        id: JString<'local>,
    ) -> jstring {
        let id: String = env.get_string(&id)?.into();
        env.new_string(id).map(JString::into_raw)
    }

    /// `Probe.boundaryError()`: a JNI call that cannot succeed, so the guard must translate it
    /// to a Java exception and the method's sentinel.
    fn Java_rs_pushups_ci_Probe_boundaryError<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
    ) -> jstring {
        let null = JString::default();
        let _string = env.get_string(&null)?;
        Ok(std::ptr::null_mut())
    }

    /// `Probe.boundaryPanic(id)`: an ordinary unwinding panic.
    fn Java_rs_pushups_ci_Probe_boundaryPanic<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
        id: JString<'local>,
    ) -> jstring {
        let id: String = env.get_string(&id)?.into();
        panic!("jni boundary panic {id}");
    }

    /// `Probe.boundaryNested(id)`: records and returns a line only when its body runs, which
    /// the guard must not do while `boundaryPending` leaves an exception pending.
    fn Java_rs_pushups_ci_Probe_boundaryNested<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
        id: JString<'local>,
    ) -> jstring {
        let id: String = env.get_string(&id)?.into();
        EVENTS.lock().push(format!("boundary nested {id}"));
        env.new_string(format!("boundary nested {id}")).map(JString::into_raw)
    }

    /// `Probe.boundaryPending(id)`: leaves a distinctive `IllegalStateException` pending, then
    /// enters `boundaryNested`'s generated entrypoint through it.
    fn Java_rs_pushups_ci_Probe_boundaryPending<'local>(
        env: JNIEnv<'local>,
        class: JClass<'local>,
        id: JString<'local>,
    ) -> jstring {
        let text: String = env.get_string(&id)?.into();
        env.throw_new(
            "java/lang/IllegalStateException",
            format!("jni boundary pending {text}"),
        )?;
        // SAFETY: the duplicate wraps this thread's environment, and the nested entrypoint
        // creates no local reference that outlives this frame.
        let nested = unsafe { env.unsafe_clone() };
        // SAFETY: the nested entrypoint runs on this thread with live local references of this frame.
        Ok(unsafe { Java_rs_pushups_ci_Probe_boundaryNested(nested, class, id) })
    }

    /// `Probe.boundaryPanicPending(id)`: leaves a distinctive `IllegalStateException` pending
    /// and then panics, so the guard must catch the panic and keep the original exception.
    fn Java_rs_pushups_ci_Probe_boundaryPanicPending<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
        id: JString<'local>,
    ) -> jstring {
        let id: String = env.get_string(&id)?.into();
        env.throw_new(
            "java/lang/IllegalStateException",
            format!("jni boundary panic pending {id}"),
        )?;
        panic!("jni boundary panic pending {id}");
    }

    /// `Probe.boundaryPanickingDrop(id)`: panics with a payload whose destructor panics.
    fn Java_rs_pushups_ci_Probe_boundaryPanickingDrop<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
        id: JString<'local>,
    ) -> jstring {
        let id: String = env.get_string(&id)?.into();
        let payload = PanickingPayload { id };
        panic_any(Box::new(payload));
    }

    /// `Probe.boundaryRecovery(id)`: `recovered <id>`, proving the process survives the guard's
    /// failures.
    fn Java_rs_pushups_ci_Probe_boundaryRecovery<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
        id: JString<'local>,
    ) -> jstring {
        let id: String = env.get_string(&id)?.into();
        env.new_string(format!("recovered {id}")).map(JString::into_raw)
    }
}
