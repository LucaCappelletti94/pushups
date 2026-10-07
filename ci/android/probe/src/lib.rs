//! The app library the Android instrumented tests load in place of a real app's cdylib.
//!
//! It holds the Firebase configuration and the background handler, as an app would, and exports
//! `rs.pushups.ci.Probe`, through which the Kotlin tests call `pushups`' public API and read
//! back every event as a line of text. It is built with `-Cinstrument-coverage`, and
//! `Probe.writeCoverage` writes the profile the CI job turns into LCOV.

use std::ffi::{CString, c_char, c_int};
use std::thread;

use jni::EnvUnowned;
use jni::errors::ThrowRuntimeExAndDefault;
use jni::objects::{JClass, JString};
use jni::sys::jboolean;
use parking_lot::Mutex;
use pushups::{Config, Event, Message, Token};

#[cfg(feature = "config-real")]
pushups::firebase_config!("google-services.json");
#[cfg(feature = "config-fixture")]
pushups::firebase_config!("../../../tests/fixtures/google-services.json");
#[cfg(feature = "config-mismatched")]
pushups::firebase_config!("mismatched-services.json");

/// Every event the handler received, one line each, in order.
static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Every push the background handler received, one line each, in order.
static HANDLED: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// The answer to the last `startPermission`, until `permission` takes it.
static PERMISSION: Mutex<Option<String>> = Mutex::new(None);

unsafe extern "C" {
    fn __llvm_profile_set_filename(name: *const c_char);
    fn __llvm_profile_write_file() -> c_int;
}

#[cfg(feature = "background-handler")]
#[pushups::background_handler]
fn on_push(_context: pushups::AndroidContext, message: Message) {
    HANDLED.lock().push(describe_message(&message));
}

fn describe(event: &Event) -> String {
    match event {
        Event::Token(Token::Fcm(token)) => format!("token {token}"),
        Event::Token(other) => format!("token {other:?}"),
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
fn java_string<'caller>(
    mut unowned: EnvUnowned<'caller>,
    text: Option<String>,
) -> JString<'caller> {
    unowned
        .with_env(|env| -> jni::errors::Result<JString<'caller>> {
            match text {
                Some(text) => env.new_string(text),
                None => Ok(JString::default()),
            }
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// `Probe.install()`: `null`, or the error of `pushups::install`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_rs_pushups_ci_Probe_install<'caller>(
    unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
) -> JString<'caller> {
    java_string(
        unowned,
        pushups::install(Config::new())
            .err()
            .map(|error| error.to_string()),
    )
}

/// `Probe.setHandler()`: records every event from now on.
#[unsafe(no_mangle)]
pub extern "system" fn Java_rs_pushups_ci_Probe_setHandler<'caller>(
    _unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
) {
    pushups::set_handler(|event| EVENTS.lock().push(describe(&event)));
}

/// `Probe.register()`: `null`, or the error of `pushups::register`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_rs_pushups_ci_Probe_register<'caller>(
    unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
) -> JString<'caller> {
    java_string(
        unowned,
        pushups::register().err().map(|error| error.to_string()),
    )
}

/// `Probe.startPermission()`: asks for the permission on a thread of its own, since the test
/// thread has to answer the prompt.
#[unsafe(no_mangle)]
pub extern "system" fn Java_rs_pushups_ci_Probe_startPermission<'caller>(
    _unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
) {
    PERMISSION.lock().take();
    thread::spawn(|| {
        let answer = match futures_executor::block_on(pushups::request_permission()) {
            Ok(permission) => format!("{permission:?}"),
            Err(error) => format!("error {error}"),
        };
        *PERMISSION.lock() = Some(answer);
    });
}

/// `Probe.permission()`: the answer to `startPermission`, or `null` while it is pending.
#[unsafe(no_mangle)]
pub extern "system" fn Java_rs_pushups_ci_Probe_permission<'caller>(
    unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
) -> JString<'caller> {
    java_string(unowned, PERMISSION.lock().take())
}

/// `Probe.events()`: every event so far, one per line.
#[unsafe(no_mangle)]
pub extern "system" fn Java_rs_pushups_ci_Probe_events<'caller>(
    unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
) -> JString<'caller> {
    java_string(unowned, Some(EVENTS.lock().join("\n")))
}

/// `Probe.handled()`: every push the background handler received, one per line.
#[unsafe(no_mangle)]
pub extern "system" fn Java_rs_pushups_ci_Probe_handled<'caller>(
    unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
) -> JString<'caller> {
    java_string(unowned, Some(HANDLED.lock().join("\n")))
}

/// `Probe.writeCoverage(path)`: writes the coverage profile to `path`, `true` on success.
#[unsafe(no_mangle)]
pub extern "system" fn Java_rs_pushups_ci_Probe_writeCoverage<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    path: JString<'caller>,
) -> jboolean {
    unowned
        .with_env(|env| -> jni::errors::Result<bool> {
            let path = CString::new(path.try_to_string(env)?).unwrap_or_default();
            // SAFETY: `path` is a NUL-terminated string that outlives both calls, and the
            // profiling runtime copies the name before the write returns.
            let written = unsafe {
                __llvm_profile_set_filename(path.as_ptr());
                __llvm_profile_write_file()
            };
            Ok(written == 0)
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}
