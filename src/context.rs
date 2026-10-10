#[cfg(target_os = "android")]
use std::ffi::c_void;

/// What a [`background_handler`](crate::background_handler) runs with, on every platform.
///
/// On Android it carries the raw `JavaVM` and `Context`, in the shape `ndk_context::AndroidContext` uses, so the handler can hand them to `jni::JavaVM::from_raw` or to `ndk_context::initialize_android_context`. They are valid only until the handler returns, and the `Context` is a local reference of the calling thread. On Apple it carries nothing yet.
#[derive(Debug, Clone, Copy)]
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
