//! Receives pushes through `pushups` under a plain tao app, scoped to iOS and macOS since
//! `Config::new()` cannot install on Linux or Windows, with tao from crates.io or patched to
//! tao#1364, and logs every step with its wall-clock time to standard output and to
//! `pushups-example.log` in the temporary directory.

mod screen;

use std::rc::Rc;

use pushups::{Config, Context, Event, Message};
use pushups_example_support as support;
use tao::event::{ElementState, Event as TaoEvent, TouchPhase, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;

use crate::screen::Screen;

/// What the event loop wakes for, a push event or the answer to the permission prompt.
enum Update {
    Push(Event),
    Permission(String),
}

fn main() {
    match pushups::install(Config::new()) {
        Ok(()) => log(&format!("installed at_ms={}", support::now_ms())),
        Err(error) => log(&format!("install failed: {error}")),
    }
    let event_loop = EventLoopBuilder::<Update>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let pushes = proxy.clone();
    pushups::set_handler(move |event| {
        // A closed loop means the app is exiting.
        let _ = pushes.send_event(Update::Push(event));
    });
    if let Err(error) = pushups::register() {
        log(&format!("register failed: {error}"));
    }
    // Holds the window and its drawing for the life of the loop.
    let mut screen = None;
    // A prompt is open, so another click asks nothing.
    let mut asking = false;
    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::Wait;
        let mut ask = false;
        match event {
            TaoEvent::NewEvents(tao::event::StartCause::Init) => {
                screen = WindowBuilder::new()
                    .with_title("pushups tao example")
                    .build(target)
                    .map_err(|error| log(&format!("window failed: {error}")))
                    .ok()
                    .and_then(|window| {
                        Screen::new(Rc::new(window))
                            .map_err(|error| log(&format!("drawing surface failed: {error}")))
                            .ok()
                    });
                if let Some(screen) = &mut screen {
                    screen.push("Click, tap or press a key to allow notifications".to_owned());
                }
                log(&format!("ui ready at_ms={}", support::now_ms()));
            }
            TaoEvent::UserEvent(Update::Push(event)) => {
                log(&support::describe(&event));
                if let Some(screen) = &mut screen {
                    screen.push(support::shown(&event));
                }
                if let Err(error) = support::register_again(&event) {
                    log(&format!("register failed: {error}"));
                }
            }
            TaoEvent::UserEvent(Update::Permission(answer)) => {
                log(&format!("permission {answer}"));
                if let Some(screen) = &mut screen {
                    screen.push(format!("Notifications: {answer}"));
                }
                asking = false;
            }
            TaoEvent::RedrawRequested(_) => {
                if let Some(screen) = &mut screen {
                    screen.draw();
                }
            }
            TaoEvent::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
                // A mouse button, a touch or a key, the user's action the prompt answers.
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    ..
                } => ask = true,
                WindowEvent::Touch(touch) if touch.phase == TouchPhase::Started => ask = true,
                WindowEvent::KeyboardInput { event, .. }
                    if event.state == ElementState::Pressed && !event.repeat =>
                {
                    ask = true;
                }
                _ => {}
            },
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
        if ask && !asking {
            asking = true;
            let proxy = proxy.clone();
            // A thread of its own, outside any async runtime, so blocking on the prompt is safe.
            std::thread::spawn(move || {
                let answer =
                    support::permission(futures_executor::block_on(pushups::request_permission()));
                let _ = proxy.send_event(Update::Permission(answer));
            });
        }
    });
}

/// Runs off the main thread for every push the system delivers.
#[pushups::background_handler]
fn on_push(_context: Context, message: Message) {
    support::handle_push(&message, log);
}

/// Runs in the process the push woke, and shows the push as a desktop notification on Linux and
/// Windows, before the background handler runs.
#[cfg(any(target_os = "linux", target_os = "windows"))]
#[pushups::rendering_handler]
fn render(
    _context: pushups::RenderingContext,
    message: Message,
) -> Result<pushups::Rendering, pushups::Error> {
    Ok(support::rendered(&message).into())
}


fn log(line: &str) {
    support::log("pushups-tao-example", line);
}
