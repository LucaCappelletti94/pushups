//! Receives pushes through `pushups` under winit 0.31, which installs no app delegate on Apple, and logs every step with its wall-clock time to standard output and to `pushups-example.log` in the temporary directory.

use std::io::Write;
use std::sync::mpsc::{Receiver, channel};

use pushups::{Config, Context, Event, Message};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match pushups::install(Config::new()) {
        Ok(()) => log(&format!("installed at_ms={}", now_ms())),
        Err(error) => log(&format!("install failed: {error}")),
    }
    let event_loop = EventLoop::new()?;
    let proxy = event_loop.create_proxy();
    let (sender, events) = channel();
    pushups::set_handler(move |event| {
        // A closed channel means the app is exiting.
        let _ = sender.send(event);
        proxy.wake_up();
    });
    if let Err(error) = pushups::register() {
        log(&format!("register failed: {error}"));
    }
    std::thread::spawn(|| {
        let answer = futures_executor::block_on(pushups::request_permission());
        log(&format!("permission {answer:?}"));
    });
    Ok(event_loop.run_app(App {
        window: None,
        events,
    })?)
}

struct App {
    window: Option<Box<dyn Window>>,
    events: Receiver<Event>,
}

impl ApplicationHandler for App {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        let attributes = WindowAttributes::default().with_title("pushups winit example");
        match event_loop.create_window(attributes) {
            Ok(window) => self.window = Some(window),
            Err(error) => log(&format!("window failed: {error}")),
        }
        log(&format!("ui ready at_ms={}", now_ms()));
    }

    fn proxy_wake_up(&mut self, _event_loop: &dyn ActiveEventLoop) {
        for event in self.events.try_iter() {
            log(&describe(&event));
        }
    }

    fn window_event(&mut self, event_loop: &dyn ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if event == WindowEvent::CloseRequested {
            event_loop.exit();
        }
    }
}

fn describe(event: &Event) -> String {
    let at = now_ms();
    match event {
        Event::Token(pushups::Token::Apns(token)) => {
            let hex: String = token.iter().map(|byte| format!("{byte:02x}")).collect();
            format!("ui token at_ms={at} apns={hex}")
        }
        Event::Token(token) => format!("ui token at_ms={at} {token:?}"),
        Event::RegistrationFailed(error) => format!("ui registration failed at_ms={at} {error}"),
        Event::Message(message) => format!(
            "ui message at_ms={at} started_app={} payload={}",
            message.started_app,
            String::from_utf8_lossy(&message.payload)
        ),
        Event::MessagesDropped => format!("ui messages dropped at_ms={at}"),
    }
}

/// Runs off the main thread for every push the system delivers.
#[pushups::background_handler]
fn on_push(_context: Context, message: Message) {
    log(&format!(
        "handler started_app={} entered_at_ms={} payload={}",
        message.started_app,
        now_ms(),
        String::from_utf8_lossy(&message.payload)
    ));
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis())
}

/// Prints the line, and appends it to `pushups-example.log` in the temporary directory, which outlives a process the system launched with nothing reading its output.
fn log(line: &str) {
    let line = format!("pushups-winit-example {line}");
    println!("{line}");
    let path = std::env::temp_dir().join("pushups-example.log");
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        // One write per line, so lines from the handler thread and the UI never interleave.
        let _ = file.write_all(format!("{line}\n").as_bytes());
    }
}
