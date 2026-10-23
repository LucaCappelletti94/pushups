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
        let _ = std::panic::catch_unwind(|| handler(Context::default(), message));
    }
}

#[cfg(any(target_os = "ios", target_os = "macos"))]
pub mod ios {
    use core::ffi::c_void;

    use crate::{Error, Message, Notification, RenderingContext};

    /// The adapter's completion thunk. The machine settles the request exactly once, with
    /// the settled outcome and, for a rendered alert, the transform's fields.
    ///
    /// `token` is the adapter's retained completion box, which the thunk releases after its
    /// one call. Each string pointer is null or `*_len` readable UTF-8 bytes, alive for the
    /// call, and a null body on a rendered alert clears the body.
    pub type Finish = unsafe extern "C" fn(
        token: *mut c_void,
        outcome: i32,
        marker: *const u8,
        marker_len: usize,
        title: *const u8,
        title_len: usize,
        body: *const u8,
        body_len: usize,
        tag: *const u8,
        tag_len: usize,
        data: *const u8,
        data_len: usize,
        navigate: *const u8,
        navigate_len: usize,
    );

    /// Allocates the service handle released by `deallocated`.
    #[must_use]
    pub fn create() -> *mut c_void {
        crate::apple::service::create()
    }

    /// Starts the rendering request for the alert the adapter received, superseding a pending
    /// one, under the remaining budget the adapter measured, and runs the app's transform.
    ///
    /// # Safety
    ///
    /// `service` is a live, aligned handle from `create` that cannot be destroyed during this call. `user_info` points to `user_info_len` readable bytes. `finish` may run on any thread and owns the retained `token` until its single call.
    pub unsafe fn receive(
        service: *const c_void,
        user_info: *const u8,
        user_info_len: usize,
        budget_nanos: u64,
        finish: Finish,
        token: *mut c_void,
        handler: fn(RenderingContext, Message) -> Result<Notification, Error>,
    ) {
        // SAFETY: the caller passed a live handle, a live thunk and a payload of
        // `user_info_len` bytes.
        unsafe {
            crate::apple::service::receive(
                service,
                user_info,
                user_info_len,
                budget_nanos,
                finish,
                token,
                handler,
            );
        }
    }

    /// The platform's `serviceExtensionTimeWillExpire` for the instance's handle.
    ///
    /// # Safety
    ///
    /// `service` is a live handle from `create`.
    pub unsafe fn expired(service: *const c_void) {
        // SAFETY: the caller passed a live handle.
        unsafe { crate::apple::service::expired(service) }
    }

    /// Settles the pending request and destroys the service handle.
    ///
    /// # Safety
    ///
    /// `service` is the live handle from `create`, exclusively released once with no concurrent calls.
    pub unsafe fn deallocated(service: *mut c_void) {
        // SAFETY: the caller released the handle exactly once.
        unsafe { crate::apple::service::deallocated(service) }
    }
}

#[cfg(target_os = "android")]
pub mod android {
    pub use crate::__pushups_android_exports as exports;
    pub use crate::android_boundary::boundary;
    pub use jni;

    use jni::objects::{JByteArray, JObject, JString};
    use jni::{JNIEnv, sys::jobjectArray};

    pub use super::FirebaseClient;
    use crate::{Context, Message};

    /// Returns the selected client's values as a Java-owned array in the caller's native frame.
    ///
    /// # Errors
    ///
    /// Returns JNI conversion or allocation errors.
    pub fn firebase_values(
        env: &mut JNIEnv<'_>,
        package_name: &JString<'_>,
        clients: &[FirebaseClient],
    ) -> jni::errors::Result<jobjectArray> {
        let package_name: String = env.get_string(package_name)?.into();
        let Some(client) = super::select_client(clients, &package_name) else {
            return Ok(std::ptr::null_mut());
        };
        let values = [
            client.application_id,
            client.api_key,
            client.gcm_sender_id,
            client.project_id,
        ];
        let array = env.new_object_array(4, "java/lang/String", JObject::null())?;
        for (index, value) in (0..).zip(values) {
            let value = env.new_string(value)?;
            env.set_object_array_element(&array, index, &value)?;
        }
        Ok(array.into_raw())
    }

    /// Runs the handler with owned payload bytes and raw context handles valid until the callback returns.
    ///
    /// # Errors
    ///
    /// Returns JNI conversion errors.
    pub fn run_background_handler(
        env: &mut JNIEnv<'_>,
        context: &JObject<'_>,
        payload: &JByteArray<'_>,
        started_app: bool,
        handler: fn(Context, Message),
    ) -> jni::errors::Result<()> {
        let payload = env.convert_byte_array(payload)?;
        let vm = env.get_java_vm()?;
        let context = Context {
            vm: vm.get_java_vm_pointer().cast(),
            context: context.as_raw().cast(),
        };
        handler(
            context,
            Message {
                payload,
                started_app,
            },
        );
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub use linkme;

/// The app's background handler, which `#[background_handler]` adds on Linux and Windows.
#[cfg(any(target_os = "linux", target_os = "windows"))]
#[linkme::distributed_slice]
pub static BACKGROUND_HANDLERS: [fn(crate::Context, crate::Message)];

/// Runs the app's background handler for one push on Linux and Windows, if it has one, once the
/// push is persisted or delivered. A panic in the handler is caught here, after the panic hook
/// reported it.
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) fn run_background_handler(message: crate::Message) {
    if let Some(handler) = BACKGROUND_HANDLERS.first() {
        let _ = std::panic::catch_unwind(|| handler(crate::Context::default(), message));
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
