#[cfg(not(target_os = "android"))]
use std::marker::PhantomData;

#[cfg(target_os = "android")]
use jni::{JNIEnv, objects::JObject};

/// What a [`background_handler`](crate::background_handler) runs with, borrowed for that one call on its thread.
///
/// ```
/// # #[cfg(not(target_os = "android"))] {
/// use pushups::{Context, Message};
///
/// fn on_push(_context: Context<'_>, message: Message) -> usize {
///     message.payload.len()
/// }
///
/// let message = Message { payload: b"{}".to_vec(), started_app: false };
/// assert_eq!(on_push(Context::default(), message), 2);
/// # }
/// ```
#[derive(Debug, Clone, Copy)]
#[cfg_attr(not(target_os = "android"), derive(Default))]
pub struct Context<'a> {
    #[cfg(target_os = "android")]
    env: &'a JNIEnv<'a>,
    #[cfg(target_os = "android")]
    context: &'a JObject<'a>,
    #[cfg(not(target_os = "android"))]
    _callback: PhantomData<(&'a (), *const ())>,
}

#[cfg(target_os = "android")]
impl<'a> Context<'a> {
    pub(crate) fn new(env: &'a JNIEnv<'a>, context: &'a JObject<'a>) -> Self {
        Self { env, context }
    }
}

#[cfg(target_os = "android")]
#[cfg_attr(docsrs, doc(cfg(target_os = "android")))]
impl Context<'_> {
    /// Runs `body` with the callback's JNI environment and the `android.content.Context` the push arrived on.
    pub fn with_jni<T>(
        &self,
        body: impl for<'local> FnOnce(&mut JNIEnv<'local>, &JObject<'local>) -> T,
    ) -> T {
        // SAFETY: the clone stays on the callback's thread inside its frame, and `body` cannot return its local references.
        let mut env = unsafe { self.env.unsafe_clone() };
        body(&mut env, self.context)
    }
}
