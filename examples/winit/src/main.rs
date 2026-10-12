//! Receives pushes through `pushups` under winit 0.31, which installs no app delegate on Apple,
//! and logs every step with its wall-clock time to standard output and to `pushups-example.log`
//! in the temporary directory, or to the console on the web.

mod app;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::install();
    app::run(winit::event_loop::EventLoop::new()?)
}

/// The web entry, which Trunk builds from this target. In the service worker it serves the Rust
/// handler, and in the page it installs pushups and runs the winit event loop.
#[cfg(target_arch = "wasm32")]
fn main() {
    if pushups::in_service_worker() {
        if let Err(error) = pushups::serve_service_worker(app::notification_for) {
            app::log(&format!("serving the service worker failed: {error}"));
        }
        return;
    }
    app::install();
    match winit::event_loop::EventLoop::new() {
        // The event loop does not block on the web, and keeps running after `main` returns.
        Ok(event_loop) => {
            if let Err(error) = app::run(event_loop) {
                app::log(&format!("event loop failed: {error}"));
            }
        }
        Err(error) => app::log(&format!("event loop failed: {error}")),
    }
}
