//! Panic containment and scoped thread access for Android JNI calls.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use jni::{JNIEnv, JavaVM};

mod sealed {
    pub trait Sealed {}
}

/// A JNI return value with a non-panicking failure sentinel.
pub trait Return: sealed::Sealed {
    /// The value returned while a Java exception is pending.
    fn failure() -> Self;
}

macro_rules! primitive_returns {
    ($($kind:ty => $value:expr),* $(,)?) => {
        $(
            impl sealed::Sealed for $kind {}
            impl Return for $kind {
                fn failure() -> Self { $value }
            }
        )*
    };
}

primitive_returns! {
    () => (),
    u8 => 0,
    i8 => 0,
    u16 => 0,
    i16 => 0,
    i32 => 0,
    i64 => 0,
    f32 => 0.0,
    f64 => 0.0,
    jni::sys::jobject => std::ptr::null_mut(),
}

/// Runs a native callback without replacing pending exceptions or unwinding into Java.
pub fn boundary<'local, T: Return>(
    env: &mut JNIEnv<'local>,
    body: impl FnOnce(&mut JNIEnv<'local>) -> jni::errors::Result<T>,
) -> T {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if pending(env) {
            return T::failure();
        }
        match body(env) {
            Ok(value) if !pending(env) => value,
            Ok(_) => T::failure(),
            Err(error) => {
                if !pending(env) {
                    raise(env, &error.to_string());
                }
                T::failure()
            }
        }
    }));
    match result {
        Ok(value) => value,
        Err(payload) => {
            discard_panic(payload);
            let translated = catch_unwind(AssertUnwindSafe(|| {
                raise(env, "Rust native callback panicked");
            }));
            if let Err(payload) = translated {
                std::mem::forget(payload);
                env.fatal_error("JNI panic translation failed");
            }
            T::failure()
        }
    }
}

fn pending(env: &JNIEnv<'_>) -> bool {
    env.exception_check()
        .unwrap_or_else(|_| env.fatal_error("JNI exception check failed"))
}

fn raise(env: &mut JNIEnv<'_>, message: &str) {
    if !pending(env)
        && env
            .throw_new("java/lang/RuntimeException", message)
            .is_err()
        && !pending(env)
    {
        env.fatal_error("JNI exception creation failed");
    }
}

fn discard_panic(payload: Box<dyn Any + Send>) {
    if let Err(secondary) = catch_unwind(AssertUnwindSafe(|| drop(payload))) {
        std::mem::forget(secondary);
    }
}

/// Attaches this thread and releases its local frame before returning or resuming a panic.
pub(crate) fn with_attached<T, E>(
    vm: &JavaVM,
    body: impl for<'local> FnOnce(&mut JNIEnv<'local>) -> Result<T, E>,
) -> Result<T, E>
where
    E: From<jni::errors::Error>,
{
    let mut env = vm.attach_current_thread()?;
    if pending(&env) {
        return Err(jni::errors::Error::JavaException.into());
    }
    let result = env.with_local_frame(16, |env| {
        Ok::<_, E>(catch_unwind(AssertUnwindSafe(|| body(env))))
    })?;
    match result {
        Ok(result) => result,
        Err(payload) => resume_unwind(payload),
    }
}

/// Emits native entrypoints whose arguments and returned references belong to the JVM's calling frame.
#[doc(hidden)]
#[macro_export]
macro_rules! __pushups_android_exports {
    ($(
        $(#[$attribute:meta])*
        fn $name:ident<$local:lifetime>(
            $env:ident: $environment:ty $(, $argument:ident: $kind:ty)* $(,)?
        ) -> $returned:ty $body:block
    )*) => {
        $(
            $(#[$attribute])*
            #[doc = "# Safety"]
            #[doc = "The JVM must supply arguments valid on this thread for its active native frame."]
            #[unsafe(no_mangle)]
            pub unsafe extern "system" fn $name<$local>(
                mut $env: $environment, $($argument: $kind),*
            ) -> $returned {
                $crate::__private::android::boundary(&mut $env, |$env| $body)
            }
        )*
    };
}
