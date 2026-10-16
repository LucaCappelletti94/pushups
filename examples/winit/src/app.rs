//! The app both entry points run, `main` on desktop, iOS and the web and `android_main` on Android.

use std::rc::Rc;
use std::sync::mpsc::{Receiver, channel};

use pushups::{Config, Context, Event, Message, VapidPublicKey};
use pushups_example_support as support;

// The Linux and Windows sub-configs, unused on the web, which takes its key for `WebPushConfig` instead.
#[cfg(not(target_arch = "wasm32"))]
use pushups::{LinuxConfig, UnifiedPushConfig, WnsConfig};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

use crate::screen::Screen;

/// Installs `pushups`, before the event loop exists.
pub(crate) fn install() {
    match pushups::install(config()) {
        Ok(()) => log(&format!("installed at_ms={}", support::now_ms())),
        Err(error) => log(&format!("install failed: {error}")),
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// The app's configuration from build-time keys, `PUSHUPS_VAPID_PUBLIC_KEY` (base64url) for UnifiedPush on Linux and Android and `PUSHUPS_WNS_REMOTE_ID` for Windows.
fn config() -> Config {
    // The bundle identifier, which also names the app on the Linux session bus.
    let mut config = Config::new().linux(LinuxConfig::new("rs.pushups.example"));
    if let Some(remote_id) = option_env!("PUSHUPS_WNS_REMOTE_ID") {
        config = config.windows(WnsConfig::new(remote_id));
    }
    if let Some(key) = option_env!("PUSHUPS_VAPID_PUBLIC_KEY") {
        let key =
            VapidPublicKey::from_base64url(key).expect("PUSHUPS_VAPID_PUBLIC_KEY is a VAPID key");
        config = config.unified_push(UnifiedPushConfig::new(key));
    }
    config
}

/// The web configuration from `PUSHUPS_VAPID_PUBLIC_KEY` (base64url). `PUSHUPS_WEB_WORKER=static` registers the static worker, which `build.rs` writes from `pushups::SERVICE_WORKER`, in place of the Rust handler, which runs from the `sw.js` entry.
#[cfg(target_arch = "wasm32")]
fn config() -> Config {
    let Some(key) = option_env!("PUSHUPS_VAPID_PUBLIC_KEY") else {
        return Config::new();
    };
    let key = VapidPublicKey::from_base64url(key).expect("PUSHUPS_VAPID_PUBLIC_KEY is a VAPID key");
    let web = pushups::WebPushConfig::new(key);
    Config::new().web(match option_env!("PUSHUPS_WEB_WORKER") {
        Some("static") => web,
        _ => web.rust_handler().service_worker_path("/sw.js"),
    })
}

/// Registers, asks for permission, and runs the event loop. On the web the loop does not block
/// and keeps running in the page's event loop after `main` returns.
pub(crate) fn run(event_loop: EventLoop) -> Result<(), Box<dyn std::error::Error>> {
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
    ask_permission();
    Ok(event_loop.run_app(App::new(events))?)
}

/// Asks for permission and logs the answer, off the thread that runs the event loop.
#[cfg(not(target_arch = "wasm32"))]
fn ask_permission() {
    std::thread::spawn(|| {
        let answer = futures_executor::block_on(pushups::request_permission());
        log(&format!("permission {answer:?}"));
    });
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
                let answer = pushups::request_permission().await;
                log(&format!("permission {answer:?}"));
                let text = format!("Notifications: {answer:?}");
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
    events: Receiver<Event>,
}

impl App {
    fn new(events: Receiver<Event>) -> Self {
        Self {
            window: None,
            screen: Screen::new(),
            events,
        }
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
        for event in self.events.try_iter() {
            log(&describe(&event));
            self.screen.push(support::shown(&event));
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
            _ => {}
        }
    }
}

fn describe(event: &Event) -> String {
    let at = support::now_ms();
    match event {
        Event::Token(token) => format!("ui token at_ms={at} {}", support::token(token)),
        Event::RegistrationFailed(error) => format!("ui registration failed at_ms={at} {error}"),
        Event::Message(message) => format!(
            "ui message at_ms={at} started_app={} {} payload={}",
            message.started_app,
            support::timing(message, at),
            String::from_utf8_lossy(&message.payload)
        ),
        Event::MessagesDropped => format!("ui messages dropped at_ms={at}"),
    }
}

/// Runs off the main thread for every push the system delivers.
#[pushups::background_handler]
fn on_push(_context: Context, message: Message) {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    if let Err(error) = support::popup(&message) {
        log(&format!("popup failed: {error}"));
    }
    let entered = support::now_ms();
    log(&format!(
        "handler started_app={} entered_at_ms={entered} {} payload={}",
        message.started_app,
        support::timing(&message, entered),
        String::from_utf8_lossy(&message.payload)
    ));
}

/// Runs in the service worker for every push, and decides what the user sees.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn notification_for(message: Message) -> pushups::Notification {
    let at = support::now_ms();
    let line = support::timing(&message, at);
    log(&format!("worker push at_ms={at} {line}"));
    pushups::Notification::new(
        support::field(&message, "title").unwrap_or_else(|| "pushups from Rust".to_owned()),
    )
    .body(
        support::field(&message, "body")
            .unwrap_or_else(|| format!("pushups winit example, {line}")),
    )
    .navigate("/")
}

/// Logs the line under the prefix `ci/android/example.sh`, `ci/linux/example.sh` and `ci/web/example.mjs` read.
pub(crate) fn log(line: &str) {
    support::log("pushups-winit-example", line);
}
