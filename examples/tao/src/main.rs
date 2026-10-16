//! Receives pushes through `pushups` under a plain tao app, scoped to iOS and macOS since
//! `Config::new()` cannot install on Linux or Windows, with tao from crates.io or patched to
//! tao#1364, and logs every step with its wall-clock time to standard output and to
//! `pushups-example.log` in the temporary directory.

use pushups::{Config, Context, Event, Message};
use pushups_example_support as support;
use tao::event::{Event as TaoEvent, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;

fn main() {
    match pushups::install(Config::new()) {
        Ok(()) => log(&format!("installed at_ms={}", support::now_ms())),
        Err(error) => log(&format!("install failed: {error}")),
    }
    let event_loop = EventLoopBuilder::<Event>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    pushups::set_handler(move |event| {
        // A closed loop means the app is exiting.
        let _ = proxy.send_event(event);
    });
    if let Err(error) = pushups::register() {
        log(&format!("register failed: {error}"));
    }
    std::thread::spawn(|| {
        let answer = futures_executor::block_on(pushups::request_permission());
        log(&format!("permission {answer:?}"));
    });
    // Holds the window open for the life of the loop.
    let mut _window = None;
    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            TaoEvent::NewEvents(tao::event::StartCause::Init) => {
                _window = WindowBuilder::new()
                    .with_title("pushups tao example")
                    // The brand navy from `assets/brand`.
                    .with_background_color((0x0e, 0x18, 0x24, 0xff))
                    .build(target)
                    .map_err(|error| log(&format!("window failed: {error}")))
                    .ok();
                log(&format!("ui ready at_ms={}", support::now_ms()));
            }
            TaoEvent::UserEvent(event) => log(&describe(&event)),
            TaoEvent::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            #[cfg(feature = "tao-1364")]
            TaoEvent::PushRegistration(token) => log(&format!("tao token bytes={}", token.len())),
            #[cfg(feature = "tao-1364")]
            TaoEvent::PushRegistrationError(error) => {
                log(&format!("tao registration failed {error}"))
            }
            #[cfg(feature = "tao-1364")]
            TaoEvent::RemoteNotification { payload, .. } => log(&format!(
                "tao push payload={}",
                String::from_utf8_lossy(&payload)
            )),
            _ => {}
        }
    });
}

fn describe(event: &Event) -> String {
    let at = support::now_ms();
    match event {
        Event::Token(token) => format!("ui token at_ms={at} {}", support::token(token)),
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
        support::now_ms(),
        String::from_utf8_lossy(&message.payload)
    ));
}

fn log(line: &str) {
    support::log("pushups-tao-example", line);
}
