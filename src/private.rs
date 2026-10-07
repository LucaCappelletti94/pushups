//! What the expansions of `pushups-macros` name. Not part of the API.

/// The values of one Android client in a `google-services.json`.
#[derive(Debug)]
pub struct FirebaseClient {
    pub package_name: &'static str,
    pub application_id: &'static str,
    pub api_key: &'static str,
    pub gcm_sender_id: &'static str,
    pub project_id: &'static str,
}

/// The client registered for `package_name`, or else the only client.
#[must_use]
pub fn select_client<'a>(
    clients: &'a [FirebaseClient],
    package_name: &str,
) -> Option<&'a FirebaseClient> {
    clients
        .iter()
        .find(|client| client.package_name == package_name)
        .or(match clients {
            [only] => Some(only),
            _ => None,
        })
}

#[cfg(any(target_os = "ios", target_os = "macos"))]
pub mod apple {
    use crate::{Context, Message};

    /// Runs the app's background handler for one push, from the symbol the Apple backend finds with `dlsym`.
    ///
    /// A panic in the handler is caught here, since it must not unwind into the caller.
    ///
    /// # Safety
    ///
    /// `payload` points to `len` readable bytes, valid until this call returns.
    pub unsafe fn run_background_handler(
        payload: *const u8,
        len: usize,
        started_app: bool,
        handler: fn(Context, Message),
    ) {
        // SAFETY: the caller passes a live payload of `len` bytes.
        let payload = unsafe { std::slice::from_raw_parts(payload, len) }.to_vec();
        let message = Message {
            payload,
            started_app,
        };
        // The panic hook already reported a panic, and the push stays queued or delivered.
        let _ = std::panic::catch_unwind(|| handler(Context { _private: () }, message));
    }
}

#[cfg(target_os = "android")]
pub mod android {
    use jni::EnvUnowned;
    use jni::errors::ThrowRuntimeExAndDefault;
    use jni::objects::{JByteArray, JObjectArray, JString};
    pub use jni::sys::{JNIEnv, jboolean, jbyteArray, jclass, jobject, jobjectArray, jstring};

    pub use super::FirebaseClient;
    use crate::{Context, Message};

    /// Returns the four values of the client [`select_client`](super::select_client) picks as a
    /// `String[]`, or `null` when none fits.
    ///
    /// The array is a new local reference of the calling native method's frame, which the JVM
    /// takes over when that method returns it. A JNI failure throws a `RuntimeException` and
    /// returns `null`.
    ///
    /// # Safety
    ///
    /// `env` is the `JNIEnv*` of the native method running on this thread, and `package_name`
    /// is a `java.lang.String` local reference of that call (or `null`).
    pub unsafe fn firebase_values(
        env: *mut JNIEnv,
        package_name: jstring,
        clients: &[FirebaseClient],
    ) -> jobjectArray {
        // SAFETY: the JVM passed `env` to the native method running now.
        let mut unowned = unsafe { EnvUnowned::from_raw(env) };
        unowned
            .with_env(|env| -> jni::errors::Result<_> {
                // SAFETY: a `String` local reference of this call, wrapped once.
                let package_name = unsafe { JString::from_raw(env, package_name) };
                let package_name = package_name.try_to_string(env)?;
                let Some(client) = super::select_client(clients, &package_name) else {
                    return Ok(JObjectArray::<JString>::default());
                };
                let values = [
                    client.application_id,
                    client.api_key,
                    client.gcm_sender_id,
                    client.project_id,
                ];
                let empty = JString::from_str(env, "")?;
                let array = JObjectArray::<JString>::new(env, values.len(), &empty)?;
                for (index, value) in values.into_iter().enumerate() {
                    let value = JString::from_str(env, value)?;
                    array.set_element(env, index, &value)?;
                }
                Ok(array)
            })
            .resolve::<ThrowRuntimeExAndDefault>()
            .into_raw()
    }

    /// Runs the app's background handler for one push.
    ///
    /// The handler gets a copy of the payload and a [`Context`] holding the raw `JavaVM` and `Context`, which stay
    /// owned by the JVM and valid until this call returns. A JNI failure or a panic in the
    /// handler throws a `RuntimeException` back to the service.
    ///
    /// # Safety
    ///
    /// `env` is the `JNIEnv*` of the native method running on this thread, and `context` and
    /// `payload` are an `android.content.Context` and a `byte[]` local reference of that call.
    pub unsafe fn run_background_handler(
        env: *mut JNIEnv,
        context: jobject,
        payload: jbyteArray,
        started_app: jboolean,
        handler: fn(Context, Message),
    ) {
        // SAFETY: the JVM passed `env` to the native method running now.
        let mut unowned = unsafe { EnvUnowned::from_raw(env) };
        unowned
            .with_env(|env| -> jni::errors::Result<()> {
                // SAFETY: a `byte[]` local reference of this call, wrapped once.
                let payload = unsafe { JByteArray::from_raw(env, payload) };
                let payload = env.convert_byte_array(&payload)?;
                let vm = env.get_java_vm()?;
                let context = Context {
                    vm: vm.get_raw().cast(),
                    context: context.cast(),
                };
                handler(
                    context,
                    Message {
                        payload,
                        started_app,
                    },
                );
                Ok(())
            })
            .resolve::<ThrowRuntimeExAndDefault>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(package_name: &'static str) -> FirebaseClient {
        FirebaseClient {
            package_name,
            application_id: "1:2:android:3",
            api_key: "key",
            gcm_sender_id: "2",
            project_id: "project",
        }
    }

    #[test]
    fn the_client_registered_for_the_package_wins() {
        let clients = [client("a.b"), client("c.d")];
        assert_eq!(
            select_client(&clients, "c.d").map(|client| client.package_name),
            Some("c.d")
        );
    }

    #[test]
    fn a_single_client_serves_any_package() {
        let clients = [client("a.b")];
        assert_eq!(
            select_client(&clients, "x.y").map(|client| client.package_name),
            Some("a.b")
        );
    }

    #[test]
    fn several_clients_without_a_match_give_none() {
        let clients = [client("a.b"), client("c.d")];
        assert!(select_client(&clients, "x.y").is_none());
        assert!(select_client(&[], "x.y").is_none());
    }
}
