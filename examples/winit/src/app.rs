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

/// What the event loop wakes for, a push event or the answer to the permission prompt.
enum Update {
    Push(Event),
    /// The prompt's answer, which the web shows on its button instead.
    #[cfg(not(target_arch = "wasm32"))]
    Permission(String),
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
    ask_permission();
    Ok(event_loop.run_app(App::new(updates, sender, proxy))?)
}

/// Wires the page's button to ask for permission, since the prompt needs the click's gesture.
#[cfg(target_arch = "wasm32")]
fn ask_permission() {
    use wasm_bindgen::JsCast;
    let Some(button) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("allow"))
    else {
        return;
    };
    let wired = button.clone();
    let closure = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(
        move |_: web_sys::Event| {
            let button = wired.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let answer = support::permission(pushups::request_permission().await);
                log(&format!("permission {answer}"));
                let text = format!("Notifications: {answer}");
                button.set_text_content(Some(text.as_str()));
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
    #[cfg_attr(
        target_arch = "wasm32",
        expect(dead_code, reason = "the web asks from its button")
    )]
    asking: bool,
}

impl App {
    fn new(updates: Receiver<Update>, sender: Sender<Update>, proxy: EventLoopProxy) -> Self {
        let screen = Screen::new();
        // The web asks from its button, so only a native window explains the click.
        #[cfg(not(target_arch = "wasm32"))]
        let screen = {
            let mut screen = screen;
            screen.push("Click, tap or press a key to allow notifications".to_owned());
            screen
        };
        Self {
            window: None,
            screen,
            updates,
            sender,
            proxy,
            asking: false,
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
            let answer =
                support::permission(futures_executor::block_on(pushups::request_permission()));
            let _ = sender.send(Update::Permission(answer));
            proxy.wake_up();
        });
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

    fn proxy_wake_up(&mut self, _event_loop: &dyn ActiveEventLoop) {
        for update in self.updates.try_iter() {
            match update {
                Update::Push(event) => {
                    log(&support::describe(&event));
                    self.screen.push(support::shown(&event));
                    if let Err(error) = support::register_again(&event) {
                        log(&format!("register failed: {error}"));
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                Update::Permission(answer) => {
                    log(&format!("permission {answer}"));
                    self.screen.push(format!("Notifications: {answer}"));
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
            // A mouse button, a touch or a key, the user's action the prompt answers.
            #[cfg(not(target_arch = "wasm32"))]
            WindowEvent::PointerButton {
                state: ElementState::Pressed,
                ..
            } => self.ask_permission(),
            #[cfg(not(target_arch = "wasm32"))]
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                self.ask_permission();
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

/// Runs in the service worker for every push, and decides what the user sees.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn notification_for(message: Message) -> pushups::Notification {
    support::worker_notification(&message, log)
}

/// Logs the line under the prefix `ci/android/example.sh`, `ci/linux/example.sh` and `ci/web/example.mjs` read.
pub(crate) fn log(line: &str) {
    support::log("pushups-winit-example", line);
}
