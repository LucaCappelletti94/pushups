//! The Android build of the example, `libmain.so`, which `NativeActivity` starts through
//! `android_main`. It is empty on every other target, where `main.rs` is the app.

#[cfg(target_os = "android")]
mod app;

#[cfg(target_os = "android")]
pushups::firebase_config!("google-services.json");

/// Runs the app in the Activity android-activity starts it in.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(activity: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;

    app::install();
    let event_loop = winit::event_loop::EventLoop::builder()
        .with_android_app(activity)
        .build();
    let result = event_loop.map_err(Into::into).and_then(app::run);
    if let Err(error) = result {
        eprintln!("pushups winit example: {error}");
    }
}
