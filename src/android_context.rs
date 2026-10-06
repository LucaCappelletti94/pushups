use std::ffi::c_void;

/// The Android `JavaVM` and `Context` a [`background_handler`](crate::background_handler)
/// runs with.
///
/// Both are raw JNI pointers, the shape `ndk_context::AndroidContext` uses, so the handler can
/// hand them to `jni::JavaVM::from_raw` or to `ndk_context::initialize_android_context`. They
/// are valid only until the handler returns, and the `Context` is a local reference of the
/// calling thread.
#[derive(Debug, Clone, Copy)]
pub struct AndroidContext {
    pub(crate) vm: *mut c_void,
    pub(crate) context: *mut c_void,
}

impl AndroidContext {
    /// The process's `JavaVM*`.
    #[must_use]
    pub fn vm(&self) -> *mut c_void {
        self.vm
    }

    /// The `android.content.Context` the push arrived on, a JNI `jobject`.
    #[must_use]
    pub fn context(&self) -> *mut c_void {
        self.context
    }
}
