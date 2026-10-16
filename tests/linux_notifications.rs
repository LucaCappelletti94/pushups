//! The Linux notification scenarios, each a child process of this binary on a private session bus
//! with a fake portal or a fake notification server, the child selected by `PUSHUPS_TEST_CHILD`.
//! A tap on a closed app comes back as a process the bus starts from the activation file, which
//! inherits the bus's environment and so runs the same scenario.

#[cfg(target_os = "linux")]
fn main() {
    linux::main();
}

#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    use pushups::{Config, Error, Event, LinuxConfig, Notification};
    use zbus::zvariant::{OwnedValue, Value};

    const CHILD: &str = "PUSHUPS_TEST_CHILD";
    /// The directory the parent and its children exchange marks through.
    const MARKS: &str = "PUSHUPS_TEST_MARKS";
    const PORTAL: &str = "portal";
    const PORTAL_CLOSED: &str = "portal-closed";
    const FALLBACK: &str = "fallback";
    const NO_SERVICE: &str = "no-service";
    /// The app writes its desktop entry and shows at once, before the shell can know the entry.
    const FRESH_ENTRY: &str = "fresh-entry";
    /// The desktop entry the parent puts in place for the scenarios that need one already.
    const PACKAGED_ENTRY: &str = "[Desktop Entry]\nType=Application\nName=Packaged\n";
    const APP_ID: &str = "rs.pushups.LinuxNotifyTest";
    const APP_PATH: &str = "/rs/pushups/LinuxNotifyTest";
    const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
    const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
    const NOTIFICATIONS_NAME: &str = "org.freedesktop.Notifications";
    const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";
    /// How long a scenario may wait on a child, generous for a loaded runner.
    const BOUND: Duration = Duration::from_secs(30);
    /// How long a child listens for a tap that must not come.
    const QUIET: Duration = Duration::from_secs(2);

    pub fn main() {
        if let Ok(scenario) = std::env::var(CHILD) {
            child(&scenario);
            return;
        }
        for scenario in [PORTAL, PORTAL_CLOSED, FALLBACK, NO_SERVICE, FRESH_ENTRY] {
            run(scenario);
            println!("linux notification scenario {scenario}: ok");
        }
    }

    type Log = Arc<Mutex<Vec<String>>>;

    fn text(map: &HashMap<String, OwnedValue>, key: &str) -> String {
        map.get(key)
            .and_then(|value| <&str>::try_from(value).ok())
            .unwrap_or("-")
            .to_owned()
    }

    /// The host Registry of the fake portal.
    struct Registry(Log);

    #[expect(
        clippy::needless_pass_by_value,
        reason = "zbus hands the served object owned arguments"
    )]
    #[zbus::interface(name = "org.freedesktop.host.portal.Registry")]
    impl Registry {
        fn register(&self, app_id: String, options: HashMap<String, OwnedValue>) {
            drop(options);
            self.0
                .lock()
                .expect("the log")
                .push(format!("register {app_id}"));
        }
    }

    /// The notification interface of the fake portal.
    struct PortalNotification(Log);

    #[expect(
        clippy::needless_pass_by_value,
        reason = "zbus hands the served object owned arguments"
    )]
    #[zbus::interface(name = "org.freedesktop.portal.Notification")]
    impl PortalNotification {
        fn add_notification(&self, id: String, notification: HashMap<String, OwnedValue>) {
            self.0.lock().expect("the log").push(format!(
                "add {id} title={} body={} action={} target={}",
                text(&notification, "title"),
                text(&notification, "body"),
                text(&notification, "default-action"),
                text(&notification, "default-action-target"),
            ));
        }
    }

    /// The fake `org.freedesktop.Notifications` server, which numbers notifications from 7.
    struct Notifications(Log);

    #[expect(
        clippy::needless_pass_by_value,
        reason = "zbus hands the served object owned arguments"
    )]
    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl Notifications {
        #[expect(
            clippy::too_many_arguments,
            reason = "the specification's `Notify` takes eight arguments"
        )]
        fn notify(
            &self,
            app_name: String,
            replaces_id: u32,
            app_icon: String,
            summary: String,
            body: String,
            actions: Vec<String>,
            hints: HashMap<String, OwnedValue>,
            timeout: i32,
        ) -> u32 {
            drop((app_icon, hints, timeout));
            self.0.lock().expect("the log").push(format!(
                "notify app={app_name} replaces={replaces_id} summary={summary} body={body} actions={actions:?}"
            ));
            7
        }
    }

    /// A process killed when dropped, so a failed assertion leaves nothing running.
    struct Reaped(Child);

    impl Drop for Reaped {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// Starts `dbus-daemon` on a private session bus whose activated services see `env`. It reads
    /// activation files only from `services`, so no service of the host, such as its own portal,
    /// answers in a scenario's place.
    fn start_bus(services: &Path, env: &[(&str, &Path)]) -> (Reaped, String) {
        let config = services.with_extension("conf");
        std::fs::write(
            &config,
            format!(
                "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\" \
                 \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\n\
                 <busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen>\
                 <servicedir>{}</servicedir><policy context=\"default\">\
                 <allow send_destination=\"*\" eavesdrop=\"true\"/><allow eavesdrop=\"true\"/>\
                 <allow own=\"*\"/></policy></busconfig>\n",
                services.display()
            ),
        )
        .expect("the bus configuration is written");
        let mut command = Command::new("dbus-daemon");
        command
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address=1"])
            .stdout(Stdio::piped());
        for (name, value) in env {
            command.env(name, value);
        }
        let mut daemon = command.spawn().expect("dbus-daemon starts");
        let mut address = String::new();
        BufReader::new(daemon.stdout.take().expect("the daemon's stdout is piped"))
            .read_line(&mut address)
            .expect("dbus-daemon prints its address");
        (Reaped(daemon), address.trim().to_owned())
    }

    /// Polls `done` until it holds or [`BOUND`] passes.
    fn wait_until(mut done: impl FnMut() -> bool) -> bool {
        let started = Instant::now();
        while started.elapsed() < BOUND {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        done()
    }

    fn connect(address: &str) -> zbus::blocking::connection::Builder<'static> {
        zbus::blocking::connection::Builder::address(address).expect("the bus address parses")
    }

    fn mark(name: &str) -> PathBuf {
        Path::new(&std::env::var_os(MARKS).expect("the parent names the marks")).join(name)
    }

    fn activate(connection: &zbus::blocking::Connection, data: &str) {
        connection
            .call_method(
                Some(APP_ID),
                APP_PATH,
                Some("org.freedesktop.Application"),
                "ActivateAction",
                &(
                    "pushups-tap",
                    vec![Value::from(data)],
                    HashMap::<&str, Value<'_>>::new(),
                ),
            )
            .expect("ActivateAction reaches the app");
    }

    fn run(scenario: &'static str) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let data_home = dir.path().join("data");
        let marks = dir.path().join("marks");
        std::fs::create_dir_all(&marks).expect("the marks directory");
        let exe = std::env::current_exe().expect("this binary's path");
        let scenario_path = PathBuf::from(scenario);
        // The bus starts the app for a tap with this environment, so the started process runs the scenario too.
        let services = data_home.join("dbus-1").join("services");
        std::fs::create_dir_all(&services).expect("the service directory");
        let (_daemon, address) = start_bus(
            &services,
            &[
                ("XDG_DATA_HOME", &data_home),
                (CHILD, &scenario_path),
                (MARKS, &marks),
            ],
        );
        let log: Log = Arc::default();
        if scenario == PORTAL {
            let applications = data_home.join("applications");
            std::fs::create_dir_all(&applications).expect("the applications directory");
            std::fs::write(
                applications.join(format!("{APP_ID}.desktop")),
                PACKAGED_ENTRY,
            )
            .expect("the packaged desktop entry");
        }
        let _portal =
            matches!(scenario, PORTAL | PORTAL_CLOSED | FALLBACK | FRESH_ENTRY).then(|| {
                let builder = connect(&address)
                    .name(PORTAL_NAME)
                    .expect("the portal name is valid")
                    .serve_at(PORTAL_PATH, PortalNotification(Arc::clone(&log)))
                    .expect("the portal notification object is served");
                // A portal older than 1.19.4 has no Registry, which sends the app to the fallback.
                let builder = if scenario == FALLBACK {
                    builder
                } else {
                    builder
                        .serve_at(PORTAL_PATH, Registry(Arc::clone(&log)))
                        .expect("the registry is served")
                };
                builder.build().expect("the portal connects")
            });
        let notifications = matches!(scenario, FALLBACK | FRESH_ENTRY).then(|| {
            connect(&address)
                .name(NOTIFICATIONS_NAME)
                .expect("the notifications name is valid")
                .serve_at(NOTIFICATIONS_PATH, Notifications(Arc::clone(&log)))
                .expect("the notifications object is served")
                .build()
                .expect("the notification server connects")
        });
        let spawn = |child_scenario: &str| {
            Reaped(
                Command::new(&exe)
                    .env(CHILD, child_scenario)
                    .env(MARKS, &marks)
                    .env("DBUS_SESSION_BUS_ADDRESS", &address)
                    .env("XDG_DATA_HOME", &data_home)
                    .env_remove("PUSHUPS_DBUS_ACTIVATED")
                    .spawn()
                    .expect("the app process starts"),
            )
        };
        let tapper = connect(&address)
            .build()
            .expect("a connection for the taps");
        let mut app = spawn(scenario);
        match scenario {
            PORTAL => check_portal(&marks, &data_home, &log, &tapper),
            PORTAL_CLOSED => {
                check_portal_closed(&mut app, &marks, &tapper);
                return;
            }
            FALLBACK => check_fallback(
                &marks,
                &log,
                &tapper,
                notifications
                    .as_ref()
                    .expect("the notification server runs"),
            ),
            FRESH_ENTRY => check_fresh_entry(&marks, &data_home, &log),
            _ => {}
        }
        let mut status = None;
        assert!(
            wait_until(|| {
                status = app.0.try_wait().expect("the app can be waited on");
                status.is_some()
            }),
            "{scenario}: the app still ran after {BOUND:?}"
        );
        assert!(
            status.is_some_and(|status| status.success()),
            "{scenario}: the app failed with {status:?}"
        );
    }

    /// The portal got the app id before the notifications, the tagged one under its tag and the
    /// untagged one under an id of its own, then two taps on the same data.
    fn check_portal(
        marks: &Path,
        data_home: &Path,
        log: &Log,
        tapper: &zbus::blocking::Connection,
    ) {
        assert!(
            wait_until(|| marks.join("shown").exists()),
            "{PORTAL}: no notification shown"
        );
        let recorded = log.lock().expect("the log").clone();
        assert_eq!(
            recorded[..2],
            [
                format!("register {APP_ID}"),
                "add tag-1 title=Title body=Body action=app.pushups-tap target=tap-data".to_owned(),
            ],
            "{PORTAL}: the portal calls"
        );
        let untagged = recorded.get(2).expect("the second notification");
        assert!(
            untagged.starts_with("add ")
                && !untagged.starts_with("add tag-1 ")
                && untagged.ends_with(" title=Untagged body=- action=- target=-"),
            "{PORTAL}: {untagged}"
        );
        assert_eq!(recorded.len(), 3, "{PORTAL}: {recorded:?}");
        let desktop = std::fs::read_to_string(
            data_home
                .join("applications")
                .join(format!("{APP_ID}.desktop")),
        )
        .expect("the desktop entry is in place");
        assert_eq!(
            desktop, PACKAGED_ENTRY,
            "install replaced the app's own desktop entry"
        );
        activate(tapper, "tap-data");
        activate(tapper, "tap-data");
    }

    /// A desktop entry this process had to write is not in the shell's index yet, which drops a
    /// portal notification for an unknown app, so the notification goes through `Notify`.
    fn check_fresh_entry(marks: &Path, data_home: &Path, log: &Log) {
        assert!(
            wait_until(|| marks.join("shown").exists()),
            "{FRESH_ENTRY}: no notification shown"
        );
        let desktop = std::fs::read_to_string(
            data_home
                .join("applications")
                .join(format!("{APP_ID}.desktop")),
        )
        .expect("install wrote the desktop entry");
        assert!(desktop.contains("NoDisplay=true"), "{desktop}");
        let recorded = log.lock().expect("the log").clone();
        assert_eq!(
            recorded,
            [format!(
                "notify app={APP_ID} replaces=0 summary=Fresh body= actions=[\"default\", \"View\"]"
            )],
            "{FRESH_ENTRY}: the calls"
        );
    }

    /// The first process installs, which writes the activation file, and exits. A tap then makes
    /// the bus start a headless process, which starts the UI, which drains the tap.
    fn check_portal_closed(app: &mut Reaped, marks: &Path, tapper: &zbus::blocking::Connection) {
        assert!(
            wait_until(|| app.0.try_wait().expect("waitable").is_some()),
            "{PORTAL_CLOSED}: the installing process still ran"
        );
        activate(tapper, "closed-data");
        assert!(
            wait_until(|| marks.join("tapped").exists()),
            "{PORTAL_CLOSED}: no UI process received the tap"
        );
        let drained = std::fs::read_to_string(marks.join("tapped")).expect("the mark");
        assert_eq!(drained, "closed-data started_app=true", "{PORTAL_CLOSED}");
    }

    /// `Notify` with the `default` action, a replacement under the tag, then a forged
    /// `ActionInvoked` from a neighbour, which delivers nothing, and the server's own.
    fn check_fallback(
        marks: &Path,
        log: &Log,
        tapper: &zbus::blocking::Connection,
        server: &zbus::blocking::Connection,
    ) {
        assert!(
            wait_until(|| marks.join("shown").exists()),
            "{FALLBACK}: no notification shown"
        );
        let recorded = log.lock().expect("the log").clone();
        assert_eq!(
            recorded,
            [
                format!(
                    "notify app={APP_ID} replaces=0 summary=Title body=Body actions=[\"default\", \"View\"]"
                ),
                format!(
                    "notify app={APP_ID} replaces=7 summary=Again body= actions=[\"default\", \"View\"]"
                ),
            ],
            "{FALLBACK}: the Notify calls"
        );
        for (sender, then) in [(tapper, Some("real")), (server, None)] {
            sender
                .emit_signal(
                    None::<&str>,
                    NOTIFICATIONS_PATH,
                    NOTIFICATIONS_NAME,
                    "ActionInvoked",
                    &(7_u32, "default"),
                )
                .expect("the signal is sent");
            if let Some(name) = then {
                std::thread::sleep(QUIET);
                std::fs::write(marks.join(name), b"").expect("the real mark");
            }
        }
    }

    fn config() -> Config {
        Config::new().linux(LinuxConfig::new(APP_ID))
    }

    /// Waits for the next message, or none within `within`.
    fn next_message(
        received: &mpsc::Receiver<Event>,
        within: Duration,
    ) -> Option<pushups::Message> {
        let deadline = Instant::now() + within;
        loop {
            match received.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Event::Message(message)) => return Some(message),
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    }

    /// The app side, run in a child process on the private bus.
    fn child(scenario: &str) {
        let activated = std::env::var_os("PUSHUPS_DBUS_ACTIVATED").is_some();
        // A process the bus started for a tap stays inside `install`, and starts the UI process.
        pushups::install(config()).expect("install connects to the private bus");
        assert!(
            !activated,
            "{scenario}: a headless process returned from install"
        );
        let (events, received) = mpsc::channel();
        pushups::set_handler(move |event| {
            let _ = events.send(event);
        });
        match scenario {
            PORTAL => {
                Notification::new("Title")
                    .body("Body")
                    .tag("tag-1")
                    .data("tap-data")
                    .show()
                    .expect("the portal shows the notification");
                Notification::new("Untagged")
                    .show()
                    .expect("the portal shows the second notification");
                std::fs::write(mark("shown"), b"").expect("the shown mark");
                let tap = next_message(&received, BOUND).expect("the tap arrives");
                assert_eq!(tap.payload, b"tap-data");
                assert!(!tap.started_app, "a tap to a running UI started nothing");
                assert!(
                    next_message(&received, QUIET).is_none(),
                    "a second tap on the same data delivered again"
                );
            }
            PORTAL_CLOSED if !mark("installed").exists() => {
                std::fs::write(mark("installed"), b"").expect("the installed mark");
            }
            PORTAL_CLOSED => {
                let tap = next_message(&received, BOUND).expect("the UI drains the tap");
                std::fs::write(
                    mark("tapped"),
                    format!(
                        "{} started_app={}",
                        String::from_utf8_lossy(&tap.payload),
                        tap.started_app
                    ),
                )
                .expect("the tapped mark");
            }
            FALLBACK => {
                Notification::new("Title")
                    .body("Body")
                    .tag("tag-1")
                    .data("fb-data")
                    .show()
                    .expect("Notify shows the notification");
                Notification::new("Again")
                    .tag("tag-1")
                    .data("fb-data")
                    .show()
                    .expect("Notify replaces the notification");
                std::fs::write(mark("shown"), b"").expect("the shown mark");
                let tap = next_message(&received, BOUND).expect("the tap arrives");
                assert!(
                    mark("real").exists(),
                    "a forged ActionInvoked was delivered"
                );
                assert_eq!(tap.payload, b"fb-data");
                assert!(!tap.started_app);
            }
            NO_SERVICE => {
                let shown = Notification::new("Nobody").show();
                assert!(
                    matches!(shown, Err(Error::Platform(_))),
                    "show with no notification service gave {shown:?}"
                );
            }
            FRESH_ENTRY => {
                Notification::new("Fresh")
                    .data("fresh-data")
                    .show()
                    .expect("the fallback shows the notification");
                std::fs::write(mark("shown"), b"").expect("the shown mark");
            }
            other => panic!("unknown scenario {other}"),
        }
    }
}
