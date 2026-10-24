//! An Android app that forbids `unsafe` code and reaches the push's `Context` through `with_jni`.
#![forbid(unsafe_code)]

use jni::JavaVM;
use jni::objects::{GlobalRef, JString};
use pushups::{Context, Message};

#[pushups::background_handler]
fn on_push(context: Context<'_>, message: Message) {
    let kept = context.with_jni(|env, context| -> jni::errors::Result<_> {
        Ok((env.get_java_vm()?, env.new_global_ref(context)?))
    });
    if let Ok((vm, application)) = kept {
        let worker = std::thread::spawn(move || files_dir(&vm, &application));
        let _ = (worker.join(), message);
    }
}

/// Uses the kept application context on a thread the callback never ran on.
fn files_dir(vm: &JavaVM, application: &GlobalRef) -> jni::errors::Result<String> {
    let mut env = vm.attach_current_thread()?;
    let dir = env
        .call_method(application, "getFilesDir", "()Ljava/io/File;", &[])?
        .l()?;
    let path = env
        .call_method(dir, "getPath", "()Ljava/lang/String;", &[])?
        .l()?;
    Ok(env.get_string(&JString::from(path))?.into())
}

#[cfg(feature = "keep-context")]
pub fn keep(context: Context<'_>) -> Context<'static> {
    context
}

#[cfg(feature = "send-context")]
pub fn send(context: Context<'static>) {
    std::thread::spawn(move || drop(context));
}

#[cfg(feature = "return-local")]
pub fn return_local(context: Context<'_>) {
    let _local = context.with_jni(|env, _| env.new_string("escapes"));
}
