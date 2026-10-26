//! The app both entry points run, `main` on desktop, iOS and the web and `android_main` on Android.

use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};

use pushups::{Config, Context, Event, Message};
use pushups_example_support as support;

use winit::application::ApplicationHandler;
#[cfg(not(target_arch = "wasm32"))]
use winit::event::ElementState;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowAttributes, WindowId};

use crate::screen::Screen;

/// Whether the platform offers the notification settings the example opens when the
/// permission is denied.
const OPEN_SETTINGS: bool = cfg!(any(
    target_os = "ios",
    target_os = "android",
    target_os = "macos",
    target_os = "windows"
));

/// What the event loop wakes for, a push event or the permission the platform holds or the
/// prompt answered.
enum Update {
    Push(Event),
    /// The permission, which the window's drawn status block shows.
    Permission(support::PermissionUi),
}

/// The window's state line for the permission, the hint while asking is offered and the
/// hint that opens the settings while the platform refuses the permission and offers them.
fn state_line(ui: &support::PermissionUi) -> String {
    if ui.asks {
        "Click, tap or press a key to allow notifications".to_owned()
    } else if ui.denied && OPEN_SETTINGS {
        "Click, tap or press a key to open Settings".to_owned()
    } else {
        ui.label.clone()
    }
}

/// Logs the permission read and maps it to what the window shows.
#[cfg(not(target_arch = "wasm32"))]
fn note(status: Result<pushups::Permission, pushups::Error>) -> support::PermissionUi {
    log(&format!(
        "permission {}",
        support::permission(status.clone())
    ));
    support::permission_ui(status)
}

/// Installs `pushups`, before the event loop exists.
pub(crate) fn install() {
    match pushups::install(config()) {
        Ok(()) => log(&format!("installed at_ms={}", support::now_ms())),
        Err(error) => log(&format!("install failed: {error}")),
    }
}

/// The app's configuration from build-time keys, `PUSHUPS_VAPID_PUBLIC_KEY` (base64url) for UnifiedPush on Linux and Android and `PUSHUPS_WNS_REMOTE_ID` for Windows.
#[cfg(not(target_arch = "wasm32"))]
fn config() -> Config {
    support::desktop_config()
}

/// The web configuration from `PUSHUPS_VAPID_PUBLIC_KEY` (base64url). `PUSHUPS_WEB_WORKER=static` registers the static worker, which `build.rs` writes from `pushups::SERVICE_WORKER`, in place of the Rust handler, which runs from the `sw.js` entry.
#[cfg(target_arch = "wasm32")]
fn config() -> Config {
    support::web_push(Some("/sw.js")).map_or_else(Config::new, |web| Config::new().web(web))
}

/// Registers and runs the event loop. On the web the loop does not block and keeps running in
/// the page's event loop after `main` returns.
pub(crate) fn run(event_loop: EventLoop) -> Result<(), Box<dyn std::error::Error>> {
    let proxy = event_loop.create_proxy();
    let (sender, updates) = channel();
    let pushes = sender.clone();
    let waker = proxy.clone();
    pushups::set_handler(move |event| {
        // A closed channel means the app is exiting.
        let _ = pushes.send(Update::Push(event));
        waker.wake_up();
    });
    if let Err(error) = pushups::register() {
        log(&format!("register failed: {error}"));
    }
    #[cfg(target_arch = "wasm32")]
    ask_permission(sender.clone());
    Ok(event_loop.run_app(App::new(updates, sender, proxy))?)
}

/// Wires the page's button to ask for permission, since the prompt needs the click's gesture,
/// and shows the permission the platform holds, which the page re-reads when it comes back to
/// the foreground, on the page's line and on the canvas's drawn status block.
#[cfg(target_arch = "wasm32")]
fn ask_permission(sender: Sender<Update>) {
    use wasm_bindgen::JsCast;
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(button) = document.get_element_by_id("allow") else {
        return;
    };
    let Some(line) = document.get_element_by_id("permission") else {
        return;
    };
    let wired = button.clone();
    let wired_line = line.clone();
    let wired_sender = sender.clone();
    let closure = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(
        move |_: web_sys::Event| {
            let button = wired.clone();
            let line = wired_line.clone();
            let sender = wired_sender.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let status = pushups::request_permission().await;
                let answer = support::permission(status.clone());
                log(&format!("permission {answer}"));
                let ui = support::permission_ui(status);
                show(&button, &line, &ui);
                // The drawn status block shows the answer, like the native window.
                let _ = sender.send(Update::Permission(ui));
            });
        },
    );
    let function = closure
        .as_js_value()
        .clone()
        .dyn_into::<js_sys::Function>()
        .expect("the closure is a function");
    let _ = button.add_event_listener_with_callback("click", &function);
    // The listener keeps the closure alive for the life of the page, so the Rust copy must not
    // drop it.
    closure.forget();
    // The permission the platform holds, and the re-reads when the page comes back to the
    // foreground.
    let visible = button.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let status = pushups::permission_status().await;
        log(&format!(
            "permission {}",
            support::permission(status.clone())
        ));
        let ui = support::permission_ui(status);
        show(&visible, &line, &ui);
        // The drawn status block shows the state, like the native window.
        let _ = sender.send(Update::Permission(ui));
        let on_foreground = visible.clone();
        // A clone for the registration, since the closure takes the document by move.
        let listener = document.clone();
        let closure = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
            if document.visibility_state() == web_sys::VisibilityState::Visible {
                let on_foreground = on_foreground.clone();
                let line = line.clone();
                let sender = sender.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let status = pushups::permission_status().await;
                    let answer = support::permission(status.clone());
                    log(&format!("permission {answer}"));
                    let ui = support::permission_ui(status);
                    show(&on_foreground, &line, &ui);
                    let _ = sender.send(Update::Permission(ui));
                });
            }
        });
        let _ = listener
            .add_event_listener_with_callback("visibilitychange", closure.as_ref().unchecked_ref());
        // The listener keeps the closure alive for the life of the page, so the Rust copy must
        // not drop it.
        closure.forget();
    });
}

/// Shows the permission on the page, the state on its line and the button hidden while asking
/// is no longer offered.
#[cfg(target_arch = "wasm32")]
fn show(button: &web_sys::Element, line: &web_sys::Element, ui: &support::PermissionUi) {
    use wasm_bindgen::JsCast;
    line.set_text_content(Some(ui.label.as_str()));
    button
        .clone()
        .unchecked_into::<web_sys::HtmlElement>()
        .set_hidden(!ui.asks);
}

struct App {
    window: Option<Rc<dyn Window>>,
    screen: Screen,
    updates: Receiver<Update>,
    // The native prompt's answer comes back through these, while the web answers on its button.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(dead_code, reason = "the web asks from its button")
    )]
    sender: Sender<Update>,
    #[cfg_attr(
        target_arch = "wasm32",
        expect(dead_code, reason = "the web asks from its button")
    )]
    proxy: EventLoopProxy,
    /// A prompt is open, so another click asks nothing.
    asking: bool,
    /// The window offers a prompt, while the web answers on its button.
    asks: bool,
    /// The platform refuses the permission, while the platform offers its settings.
    denied: bool,
}

impl App {
    fn new(updates: Receiver<Update>, sender: Sender<Update>, proxy: EventLoopProxy) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let mut screen = Screen::new();
        #[cfg(target_arch = "wasm32")]
        let screen = Screen::new();
        let (asks, denied) = {
            // The web asks from its button, so only a native window explains the click.
            #[cfg(not(target_arch = "wasm32"))]
            {
                // The permission the platform holds, read before the loop so the window starts
                // from it.
                let ui = note(futures_executor::block_on(pushups::permission_status()));
                screen.set_state(state_line(&ui));
                (ui.asks, ui.denied)
            }
            #[cfg(target_arch = "wasm32")]
            {
                (false, false)
            }
        };
        Self {
            window: None,
            screen,
            updates,
            sender,
            proxy,
            asking: false,
            asks,
            denied,
        }
    }

    /// Asks for permission off the thread that runs the event loop, and hands the answer back to
    /// the window, unless a prompt is open already.
    #[cfg(not(target_arch = "wasm32"))]
    fn ask_permission(&mut self) {
        if self.asking {
            return;
        }
        self.asking = true;
        let (sender, proxy) = (self.sender.clone(), self.proxy.clone());
        // A thread of its own, outside any async runtime, so blocking on the prompt is safe.
        std::thread::spawn(move || {
            let ui = note(futures_executor::block_on(pushups::request_permission()));
            let _ = sender.send(Update::Permission(ui));
            proxy.wake_up();
        });
    }

    /// Opens the notification settings, while the platform refuses the permission and a
    /// prompt cannot change it.
    #[cfg(not(target_arch = "wasm32"))]
    fn open_settings(&mut self) {
        if self.asking {
            return;
        }
        self.asking = true;
        let (sender, proxy) = (self.sender.clone(), self.proxy.clone());
        // A thread of its own, outside any async runtime, so the platform's answer comes back
        // to the loop.
        std::thread::spawn(move || {
            if let Err(error) = pushups::open_notification_settings() {
                log(&format!("open settings failed: {error}"));
            }
            let ui = note(futures_executor::block_on(pushups::permission_status()));
            let _ = sender.send(Update::Permission(ui));
            proxy.wake_up();
        });
    }

    /// The user's action, the prompt while asking is offered and the settings while the
    /// platform refuses the permission and offers them.
    #[cfg(not(target_arch = "wasm32"))]
    fn click(&mut self) {
        if self.asks {
            self.ask_permission();
        } else if self.denied && OPEN_SETTINGS {
            self.open_settings();
        }
    }

    /// Adds the event to the window, the token on its status block and a push as a compact
    /// line.
    fn push_screen(&mut self, event: &Event) {
        match event {
            Event::Token(token) => self
                .screen
                .set_registration(support::transport(token), &support::shortened(token)),
            Event::Message(message) => self
                .screen
                .push(support::push_view(message, support::now_ms()).line()),
            _ => self.screen.push(support::shown(event)),
        }
    }

    /// Re-reads the permission the platform holds, off the thread that runs the event loop.
    #[cfg(not(target_arch = "wasm32"))]
    fn refresh_status(&self) {
        let (sender, proxy) = (self.sender.clone(), self.proxy.clone());
        std::thread::spawn(move || {
            let ui = note(futures_executor::block_on(pushups::permission_status()));
            let _ = sender.send(Update::Permission(ui));
            proxy.wake_up();
        });
    }

    /// Draws the permission the window shows, the hint while asking is offered and the
    /// settings while the platform refuses the permission and offers them.
    fn draw_permission(&mut self, ui: support::PermissionUi) {
        self.screen.set_state(state_line(&ui));
        self.asks = ui.asks;
        self.denied = ui.denied;
    }
}

impl ApplicationHandler for App {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        let attributes = WindowAttributes::default().with_title("pushups winit example");
        // A desktop or page window starts small, while a phone's fills the screen.
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        let attributes = attributes.with_surface_size(winit::dpi::LogicalSize::new(640.0, 400.0));
        // The page shows the canvas under its heading.
        #[cfg(target_arch = "wasm32")]
        let attributes = attributes.with_platform_attributes(Box::new(
            winit::platform::web::WindowAttributesWeb::default().with_append(true),
        ));
        match event_loop.create_window(attributes) {
            Ok(window) => {
                let window: Rc<dyn Window> = Rc::from(window);
                if let Err(error) = self.screen.attach(Rc::clone(&window)) {
                    log(&format!("drawing surface failed: {error}"));
                }
                self.window = Some(window);
            }
            Err(error) => log(&format!("window failed: {error}")),
        }
        log(&format!("ui ready at_ms={}", support::now_ms()));
    }

    fn destroy_surfaces(&mut self, _event_loop: &dyn ActiveEventLoop) {
        self.screen.detach();
        self.window = None;
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn resumed(&mut self, _event_loop: &dyn ActiveEventLoop) {
        self.refresh_status();
    }

    fn proxy_wake_up(&mut self, _event_loop: &dyn ActiveEventLoop) {
        while let Ok(update) = self.updates.try_recv() {
            match update {
                Update::Push(event) => {
                    log(&support::describe(&event));
                    self.push_screen(&event);
                    if let Err(error) = support::register_again(&event) {
                        log(&format!("register failed: {error}"));
                    }
                }
                Update::Permission(ui) => {
                    self.draw_permission(ui);
                    self.asking = false;
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.screen.draw(),
            WindowEvent::SurfaceResized(_) => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            // The window is back in the foreground, where the user may have changed the
            // permission in Settings.
            #[cfg(not(target_arch = "wasm32"))]
            WindowEvent::Focused(true) => self.refresh_status(),
            // A mouse button, a touch or a key, the user's action the prompt and the settings
            // answer.
            #[cfg(not(target_arch = "wasm32"))]
            WindowEvent::PointerButton {
                state: ElementState::Pressed,
                ..
            } => self.click(),
            #[cfg(not(target_arch = "wasm32"))]
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                self.click();
            }
            _ => {}
        }
    }
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

/// Runs in the service worker for every push, and decides what the user sees.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn notification_for(message: Message) -> pushups::Notification {
    support::worker_notification(&message, log)
}

/// Logs the line under the prefix `ci/android/example.sh`, `ci/linux/example.sh` and `ci/web/example.mjs` read.
pub(crate) fn log(line: &str) {
    support::log("pushups-winit-example", line);
}
