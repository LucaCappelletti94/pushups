#[cfg(target_os = "android")]
use std::ffi::c_void;

/// What a [`background_handler`](crate::background_handler) runs with, on every platform.
///
/// On Android it carries the raw `JavaVM` and `Context`, in the shape `ndk_context::AndroidContext` uses, so the handler can hand them to `jni::JavaVM::from_raw` or to `ndk_context::initialize_android_context`. They are valid only until the handler returns, and the `Context` is a local reference of the calling thread. Elsewhere it carries nothing yet, and `Context::default()` builds one, so a test can call a handler directly.
///
/// ```
/// # #[cfg(not(target_os = "android"))] {
/// use pushups::{Context, Message};
///
/// fn on_push(_context: Context, message: Message) -> usize {
///     message.payload.len()
/// }
///
/// let message = Message { payload: b"{}".to_vec(), started_app: false };
/// assert_eq!(on_push(Context::default(), message), 2);
/// # }
/// ```
#[derive(Debug, Clone, Copy)]
#[cfg_attr(not(target_os = "android"), derive(Default))]
pub struct Context {
    #[cfg(target_os = "android")]
    pub(crate) vm: *mut c_void,
    #[cfg(target_os = "android")]
    pub(crate) context: *mut c_void,
    #[cfg(not(target_os = "android"))]
    pub(crate) _private: (),
}

#[cfg(target_os = "android")]
#[cfg_attr(docsrs, doc(cfg(target_os = "android")))]
impl Context {
    /// The process's `JavaVM*`.
    #[must_use]
    pub fn android_vm(&self) -> *mut c_void {
        self.vm
    }

    /// The `android.content.Context` the push arrived on, a JNI `jobject`.
    #[must_use]
    pub fn android_context(&self) -> *mut c_void {
        self.context
    }
}
