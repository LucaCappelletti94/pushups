//! Receives pushes through `pushups` under winit 0.31, which installs no app delegate on Apple, and logs every step with its wall-clock time to standard output and to `pushups-example.log` in the temporary directory.

mod app;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::install();
    app::run(winit::event_loop::EventLoop::new()?)
}
