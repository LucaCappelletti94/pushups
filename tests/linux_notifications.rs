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

/// The test's rendering handler, which answers `NoAlert` to `no-alert`, unwinds on `panic` and
/// shows the payload kind as a tagged notification otherwise.
#[cfg(target_os = "linux")]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the rendering handler signature settles through a result the runner turns into an outcome"
)]
#[pushups::rendering_handler]
fn render(
    _context: pushups::RenderingContext,
    message: pushups::Message,
) -> Result<pushups::Rendering, pushups::Error> {
    let kind = String::from_utf8(message.payload).expect("the test payloads are ascii");
    match kind.as_str() {
        "no-alert" => Ok(pushups::Rendering::NoAlert),
        "panic" => panic!("the test rendering handler unwinds"),
        _ => Ok(pushups::Notification::new(&kind)
            .tag(&kind)
            .body("rendered")
            .into()),
    }
}

/// The test's background handler, which marks whether the render's notification was shown
/// before it ran.
#[cfg(target_os = "linux")]
#[pushups::background_handler]
fn on_background(_context: pushups::Context, message: pushups::Message) {
    let kind = String::from_utf8(message.payload).expect("the test payloads are ascii");
    let marks = std::env::var_os("PUSHUPS_TEST_MARKS").expect("the parent names the marks");
    let rendered = std::path::Path::new(&marks).join(format!("rendered-{kind}"));
    let order = if rendered.exists() { "after" } else { "before" };
    std::fs::write(
        std::path::Path::new(&marks).join(format!("b-{kind}")),
        order,
    )
    .expect("the background mark");
}

#[cfg(target_os = "linux")]
mod linux {
    use p256::elliptic_curve::point::AffineCoordinates;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    use pushups::{Config, Error, Event, LinuxConfig, Notification};
    use zbus::blocking::Connection;
    use zbus::zvariant::{OwnedValue, Structure, Value};

    const CHILD: &str = "PUSHUPS_TEST_CHILD";
    /// The directory the parent and its children exchange marks through.
    const MARKS: &str = "PUSHUPS_TEST_MARKS";
    const PORTAL: &str = "portal";
    const PORTAL_CLOSED: &str = "portal-closed";
    /// A launcher opens a closed app, which starts the UI with nothing to deliver.
    const LAUNCHED: &str = "launched";
    /// A portal with the Registry and no notification backend.
    const NO_BACKEND: &str = "no-backend";
    const FALLBACK: &str = "fallback";
    const NO_SERVICE: &str = "no-service";
    /// The app writes its desktop entry and shows at once, before the shell can know the entry.
    const FRESH_ENTRY: &str = "fresh-entry";
    /// A notification server that never answers `Notify`.
    const HUNG: &str = "hung";
    /// An app with a display name and icon writes them into its entry and passes them to `Notify`.
    const NAMED: &str = "named";
    /// An entry this crate wrote with other contents, which `install` rewrites and keeps the portal for.
    const STALE_ENTRY: &str = "stale-entry";
    /// A push rendered by the app's rendering handler before the background handler runs it.
    const RENDERING: &str = "rendering";
    /// The desktop entry the parent puts in place for the scenarios that need one already.
    const PACKAGED_ENTRY: &str = "[Desktop Entry]\nType=Application\nName=Packaged\n";
    /// An entry an earlier run of this crate wrote, before the app had a display name.
    const STALE_MARKED_ENTRY: &str =
        "[Desktop Entry]\nType=Application\nName=rs.pushups.LinuxNotifyTest\nX-Pushups=true\n";
    const APP_NAME: &str = "Notify Test";
    const APP_ID: &str = "rs.pushups.LinuxNotifyTest";
    const APP_PATH: &str = "/rs/pushups/LinuxNotifyTest";
    const APPLICATION: &str = "org.freedesktop.Application";
    const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
    const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
    const NOTIFICATIONS_NAME: &str = "org.freedesktop.Notifications";
    const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";
    /// How long a scenario may wait on a child, generous for a loaded runner.
    const BOUND: Duration = Duration::from_secs(30);
    /// How long a child listens for a tap that must not come.
    const QUIET: Duration = Duration::from_secs(2);
    /// How long `show` waits for the notification service.
    const SHOW_BOUND: Duration = Duration::from_secs(5);

    pub fn main() {
        if let Ok(scenario) = std::env::var(CHILD) {
            child(&scenario);
            return;
        }
        for scenario in [
            PORTAL,
            PORTAL_CLOSED,
            LAUNCHED,
            NO_BACKEND,
            FALLBACK,
            NO_SERVICE,
            FRESH_ENTRY,
            HUNG,
            NAMED,
            STALE_ENTRY,
            RENDERING,
        ] {
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

    /// The kind of a serialized portal icon, `themed` or `file-descriptor`.
    fn icon_kind(map: &HashMap<String, OwnedValue>) -> String {
        map.get("icon")
            .and_then(|value| Structure::try_from(value.try_clone().ok()?).ok())
            .and_then(|icon| {
                icon.fields()
                    .first()
                    .and_then(|kind| <&str>::try_from(kind).ok())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "-".to_owned())
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
                "add {id} title={} body={} action={} target={} icon={}",
                text(&notification, "title"),
                text(&notification, "body"),
                text(&notification, "default-action"),
                text(&notification, "default-action-target"),
                icon_kind(&notification),
            ));
        }
    }

    /// The fake `org.freedesktop.Notifications` server, which numbers new notifications from 7.
    struct Notifications {
        log: Log,
        next: AtomicU32,
        marks: Option<PathBuf>,
    }

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
            let _ = timeout;
            self.log.lock().expect("the log").push(format!(
                "notify app={app_name} replaces={replaces_id} summary={summary} body={body} actions={actions:?} icon={app_icon} image={}",
                text(&hints, "image-path")
            ));
            if let Some(marks) = &self.marks {
                std::fs::write(marks.join(format!("rendered-{summary}")), b"")
                    .expect("the rendered mark");
            }
            if replaces_id == 0 {
                self.next.fetch_add(1, Ordering::SeqCst)
            } else {
                replaces_id
            }
        }
    }

    /// A notification server that holds every `Notify` unanswered.
    struct Hung;

    #[expect(
        clippy::too_many_arguments,
        reason = "the specification's `Notify` takes eight arguments"
    )]
    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl Hung {
        async fn notify(
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
            drop((
                app_name,
                replaces_id,
                app_icon,
                summary,
                body,
                actions,
                hints,
                timeout,
            ));
            std::future::pending().await
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

    /// Runs the app's `action` as the portal does for a click, with `parameter` and an activation token.
    fn activate_action(
        connection: &Connection,
        action: &str,
        parameter: &[&str],
    ) -> zbus::Result<zbus::message::Message> {
        connection.call_method(
            Some(APP_ID),
            APP_PATH,
            Some(APPLICATION),
            "ActivateAction",
            &(
                action,
                parameter
                    .iter()
                    .copied()
                    .map(Value::from)
                    .collect::<Vec<_>>(),
                HashMap::from([("activation-token", Value::from("tok-1"))]),
            ),
        )
    }

    fn tap(connection: &Connection, data: &str) {
        activate_action(connection, "pushups-tap", &[data])
            .expect("ActivateAction reaches the app");
    }

    /// The fake services a scenario runs against, kept alive for its length.
    struct Services {
        _portal: Option<Connection>,
        notifications: Option<Connection>,
    }

    fn serve(scenario: &str, address: &str, log: &Log, marks: &Path) -> Services {
        let portal = matches!(
            scenario,
            PORTAL
                | PORTAL_CLOSED
                | LAUNCHED
                | NO_BACKEND
                | FALLBACK
                | FRESH_ENTRY
                | NAMED
                | STALE_ENTRY
        )
        .then(|| {
            let mut builder = connect(address)
                .name(PORTAL_NAME)
                .expect("the portal name is valid");
            if scenario != NO_BACKEND {
                builder = builder
                    .serve_at(PORTAL_PATH, PortalNotification(Arc::clone(log)))
                    .expect("the portal notification object is served");
            }
            // A portal older than 1.19.4 has no Registry, which sends the app to the fallback.
            if scenario != FALLBACK {
                builder = builder
                    .serve_at(PORTAL_PATH, Registry(Arc::clone(log)))
                    .expect("the registry is served");
            }
            builder.build().expect("the portal connects")
        });
        let notifications = match scenario {
            FALLBACK | FRESH_ENTRY | NO_BACKEND | NAMED | RENDERING => Some(
                connect(address)
                    .name(NOTIFICATIONS_NAME)
                    .expect("the notifications name is valid")
                    .serve_at(
                        NOTIFICATIONS_PATH,
                        Notifications {
                            log: Arc::clone(log),
                            next: AtomicU32::new(7),
                            marks: (scenario == RENDERING).then(|| marks.to_path_buf()),
                        },
                    )
                    .expect("the notifications object is served")
                    .build()
                    .expect("the notification server connects"),
            ),
            HUNG => Some(
                connect(address)
                    .name(NOTIFICATIONS_NAME)
                    .expect("the notifications name is valid")
                    .serve_at(NOTIFICATIONS_PATH, Hung)
                    .expect("the hung object is served")
                    .build()
                    .expect("the hung server connects"),
            ),
            _ => None,
        };
        Services {
            _portal: portal,
            notifications,
        }
    }

    fn run(scenario: &'static str) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let data_home = dir.path().join("data");
        let marks = dir.path().join("marks");
        std::fs::create_dir_all(&marks).expect("the marks directory");
        std::fs::write(marks.join("icon.png"), b"\x89PNG\r\n\x1a\n").expect("the icon file");
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
        let applications = data_home.join("applications");
        let entry = match scenario {
            PORTAL | NO_BACKEND | HUNG => Some(PACKAGED_ENTRY),
            STALE_ENTRY => Some(STALE_MARKED_ENTRY),
            _ => None,
        };
        if let Some(entry) = entry {
            std::fs::create_dir_all(&applications).expect("the applications directory");
            std::fs::write(applications.join(format!("{APP_ID}.desktop")), entry)
                .expect("the desktop entry in place");
        }
        let log: Log = Arc::default();
        let fakes = serve(scenario, &address, &log, &marks);
        let mut app = Reaped(
            Command::new(&exe)
                .env(CHILD, scenario)
                .env(MARKS, &marks)
                .env("DBUS_SESSION_BUS_ADDRESS", &address)
                .env("XDG_DATA_HOME", &data_home)
                .env_remove("PUSHUPS_DBUS_ACTIVATED")
                .env_remove("XDG_ACTIVATION_TOKEN")
                .spawn()
                .expect("the app process starts"),
        );
        let tapper = connect(&address)
            .build()
            .expect("a connection for the taps");
        match scenario {
            PORTAL => check_portal(&marks, &data_home, &log, &tapper),
            PORTAL_CLOSED | LAUNCHED => {
                check_closed(scenario, &mut app, &marks, &tapper);
                return;
            }
            NO_BACKEND => check_no_backend(&marks, &log),
            FALLBACK => check_fallback(
                &marks,
                &log,
                &tapper,
                fakes
                    .notifications
                    .as_ref()
                    .expect("the notification server runs"),
            ),
            FRESH_ENTRY => check_fresh_entry(&marks, &data_home, &log),
            NAMED => check_named(&marks, &data_home, &log),
            STALE_ENTRY => check_stale_entry(&marks, &data_home, &log),
            RENDERING => check_rendering(&marks, &data_home, &log, &address),
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

    /// The portal got the app id before the notifications, the tagged one under its tag with a
    /// themed icon and the untagged one under an id of its own with a file icon, then two taps on
    /// the same data and two actions the app refuses.
    fn check_portal(marks: &Path, data_home: &Path, log: &Log, tapper: &Connection) {
        assert!(
            wait_until(|| marks.join("shown").exists()),
            "{PORTAL}: no notification shown"
        );
        let recorded = log.lock().expect("the log").clone();
        assert_eq!(
            recorded[..2],
            [
                format!("register {APP_ID}"),
                "add tag-1 title=Title body=Body action=app.pushups-tap target=tap-data icon=themed"
                    .to_owned(),
            ],
            "{PORTAL}: the portal calls"
        );
        let untagged = recorded.get(2).expect("the second notification");
        assert!(
            untagged.starts_with("add ")
                && !untagged.starts_with("add tag-1 ")
                && untagged
                    .ends_with(" title=Untagged body=- action=- target=- icon=file-descriptor"),
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
        assert!(
            activate_action(tapper, "other", &["x"]).is_err(),
            "{PORTAL}: an action the app does not export was accepted"
        );
        assert!(
            activate_action(tapper, "pushups-tap", &[]).is_err(),
            "{PORTAL}: a tap without data was accepted"
        );
        tap(tapper, "tap-data");
        tap(tapper, "tap-data");
    }

    /// A portal whose notification interface is missing sends the notification to `Notify`.
    fn check_no_backend(marks: &Path, log: &Log) {
        assert!(
            wait_until(|| marks.join("shown").exists()),
            "{NO_BACKEND}: no notification shown"
        );
        let recorded = log.lock().expect("the log").clone();
        assert_eq!(
            recorded,
            [
                format!("register {APP_ID}"),
                format!(
                    "notify app={APP_ID} replaces=0 summary=Unbacked body= actions=[] icon= image=-"
                ),
            ],
            "{NO_BACKEND}: the calls"
        );
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
                "notify app={APP_ID} replaces=0 summary=Fresh body= actions=[\"default\", \"View\"] icon= image=-"
            )],
            "{FRESH_ENTRY}: the calls"
        );
    }

    /// The entry `install` wrote for the app id, which must exist.
    fn entry(scenario: &str, data_home: &Path) -> String {
        std::fs::read_to_string(
            data_home
                .join("applications")
                .join(format!("{APP_ID}.desktop")),
        )
        .unwrap_or_else(|error| panic!("{scenario}: no desktop entry, {error}"))
    }

    /// A fresh entry carries the display name, the icon and the crate's mark, and `Notify` names
    /// the app and its icon for a notification without one of its own.
    fn check_named(marks: &Path, data_home: &Path, log: &Log) {
        assert!(
            wait_until(|| marks.join("shown").exists()),
            "{NAMED}: no notification shown"
        );
        let icon = marks.join("icon.png");
        let icon = icon.to_string_lossy();
        let desktop = entry(NAMED, data_home);
        for line in [
            format!("Name={APP_NAME}"),
            format!("Icon={icon}"),
            "X-Pushups=true".to_owned(),
        ] {
            assert!(
                desktop.lines().any(|l| l == line),
                "{NAMED}: no {line} in {desktop}"
            );
        }
        let recorded = log.lock().expect("the log").clone();
        assert_eq!(
            recorded,
            [
                format!(
                    "notify app={APP_NAME} replaces=0 summary=Named body= actions=[] icon={icon} image=-"
                ),
                format!(
                    "notify app={APP_NAME} replaces=0 summary=Own body= actions=[] icon=/usr/share/own.png image=/usr/share/own.png"
                ),
            ],
            "{NAMED}: the calls"
        );
    }

    /// An entry this crate marked is rewritten with the display name, and counts as in place, so
    /// the notification still goes through the portal.
    fn check_stale_entry(marks: &Path, data_home: &Path, log: &Log) {
        assert!(
            wait_until(|| marks.join("shown").exists()),
            "{STALE_ENTRY}: no notification shown"
        );
        let desktop = entry(STALE_ENTRY, data_home);
        assert!(
            desktop.lines().any(|l| l == format!("Name={APP_NAME}"))
                && desktop.lines().any(|l| l == "X-Pushups=true")
                && desktop.contains("DBusActivatable=true"),
            "{STALE_ENTRY}: the entry was not rewritten, {desktop}"
        );
        let recorded = log.lock().expect("the log").clone();
        assert_eq!(
            recorded,
            [
                format!("register {APP_ID}"),
                "add stale-1 title=Stale body=- action=- target=- icon=-".to_owned(),
            ],
            "{STALE_ENTRY}: the calls"
        );
    }

    /// The push renders through the app's rendering handler before the background handler runs,
    /// `NoAlert` shows nothing, and a panicking render still lets the background handler run.
    fn check_rendering(marks: &Path, data_home: &Path, log: &Log, address: &str) {
        assert!(
            wait_until(|| marks.join("ready").exists()),
            "{RENDERING}: the child never became ready"
        );
        let dir = data_home.join(APP_ID).join("pushups");
        let token_file = std::fs::read_to_string(dir.join("token")).expect("the token file");
        let keys = std::fs::read(dir.join("keys")).expect("the keys file");
        assert_eq!(keys.len(), 48, "{RENDERING}: the keys file");
        let secret: [u8; 32] = keys[0..32].try_into().expect("the scalar is 32 bytes");
        let auth: [u8; 16] = keys[32..48]
            .try_into()
            .expect("the auth secret is 16 bytes");
        let secret = p256::SecretKey::from_bytes(&secret.into()).expect("the scalar is in range");
        let public = secret.public_key();
        let point = public.as_affine();
        let mut p256dh = [0u8; 65];
        p256dh[0] = 0x04;
        for (slot, coordinate) in p256dh[1..]
            .iter_mut()
            .zip(point.x().iter().chain(point.y().iter()))
        {
            *slot = *coordinate;
        }
        let token = pushups::Token::WebPush {
            endpoint: "https://push.example/app".to_owned(),
            p256dh,
            auth,
            expires: None,
        };
        let sender = connect(address)
            .build()
            .expect("a connection for the pushes");
        for (index, kind) in ["show", "no-alert", "panic"].iter().enumerate() {
            let sealed = pushups_sender::encrypt(&token, kind.as_bytes())
                .expect("the seal encrypts")
                .into_bytes();
            let args = HashMap::from([
                ("token", Value::from(token_file.as_str())),
                ("message", Value::from(sealed)),
                ("id", Value::from(format!("r-{index}"))),
            ]);
            sender
                .call_method(
                    Some(APP_ID),
                    "/org/unifiedpush/Connector",
                    Some("org.unifiedpush.Connector2"),
                    "Message",
                    &args,
                )
                .expect("the push reaches the connector");
        }
        std::fs::write(marks.join("done"), b"").expect("the done mark");
        let recorded = log.lock().expect("the log").clone();
        let shown: Vec<&String> = recorded
            .iter()
            .filter(|line| line.contains("summary=show"))
            .collect();
        assert_eq!(
            shown.len(),
            1,
            "{RENDERING}: the notification the render showed is not in the Notify calls, {recorded:?}"
        );
        let line = &shown[0];
        assert!(line.contains("body=rendered"), "{line}");
        assert!(
            recorded
                .iter()
                .all(|line| !line.contains("summary=no-alert") && !line.contains("summary=panic")),
            "{RENDERING}: NoAlert or the panic showed something, {recorded:?}"
        );
        for kind in ["show", "no-alert", "panic"] {
            let order =
                std::fs::read_to_string(marks.join(format!("b-{kind}"))).unwrap_or_else(|error| {
                    panic!("{RENDERING}: no background mark for {kind}, {error}")
                });
            if kind == "show" {
                assert_eq!(
                    order, "after",
                    "{RENDERING}: the background handler ran before the render finished"
                );
            } else {
                assert_eq!(order, "before", "{RENDERING}: {kind} showed something");
            }
        }
    }

    /// The first process installs, which writes the activation file, and exits. A tap, or a
    /// launcher's `Open`, then makes the bus start a headless process, which starts the UI with
    /// the call's activation token. The UI drains the tap, or finds nothing for a launch.
    fn check_closed(scenario: &str, app: &mut Reaped, marks: &Path, tapper: &Connection) {
        assert!(
            wait_until(|| app.0.try_wait().expect("waitable").is_some()),
            "{scenario}: the installing process still ran"
        );
        let expected = if scenario == PORTAL_CLOSED {
            tap(tapper, "closed-data");
            "closed-data started_app=true token=tok-1"
        } else {
            tapper
                .call_method(
                    Some(APP_ID),
                    APP_PATH,
                    Some(APPLICATION),
                    "Open",
                    &(
                        vec!["file:///tmp/a"],
                        HashMap::from([("activation-token", Value::from("tok-1"))]),
                    ),
                )
                .expect("Open reaches the app");
            "nothing token=tok-1"
        };
        assert!(
            wait_until(|| marks.join("ui").exists()),
            "{scenario}: no UI process started"
        );
        let seen = std::fs::read_to_string(marks.join("ui")).expect("the mark");
        assert_eq!(seen, expected, "{scenario}");
    }

    /// `Notify` with the `default` action, a file icon and a replacement under the tag. A forged
    /// `ActionInvoked` from a neighbour and another action key deliver nothing, the server's own
    /// delivers, and a click on a notification the server closed delivers nothing.
    fn check_fallback(marks: &Path, log: &Log, tapper: &Connection, server: &Connection) {
        assert!(
            wait_until(|| marks.join("shown").exists()),
            "{FALLBACK}: no notification shown"
        );
        let icon = marks.join("icon.png");
        let icon = icon.display();
        let recorded = log.lock().expect("the log").clone();
        assert_eq!(
            recorded,
            [
                format!(
                    "notify app={APP_ID} replaces=0 summary=Title body=Body actions=[\"default\", \"View\"] icon={icon} image={icon}"
                ),
                format!(
                    "notify app={APP_ID} replaces=7 summary=Again body= actions=[\"default\", \"View\"] icon= image=-"
                ),
                format!(
                    "notify app={APP_ID} replaces=0 summary=Closed body= actions=[\"default\", \"View\"] icon= image=-"
                ),
            ],
            "{FALLBACK}: the Notify calls"
        );
        let invoked = |sender: &Connection, id: u32, key: &str| {
            sender
                .emit_signal(
                    None::<&str>,
                    NOTIFICATIONS_PATH,
                    NOTIFICATIONS_NAME,
                    "ActionInvoked",
                    &(id, key),
                )
                .expect("the signal is sent");
        };
        invoked(tapper, 7, "default");
        invoked(server, 7, "other");
        std::thread::sleep(QUIET);
        std::fs::write(marks.join("real"), b"").expect("the real mark");
        invoked(server, 7, "default");
        server
            .emit_signal(
                None::<&str>,
                NOTIFICATIONS_PATH,
                NOTIFICATIONS_NAME,
                "NotificationClosed",
                &(8_u32, 2_u32),
            )
            .expect("the closing signal is sent");
        std::thread::sleep(QUIET / 2);
        invoked(server, 8, "default");
    }

    /// The app's configuration, with the display name and icon for the scenarios that check them
    /// and for the portal, whose packaged entry they must leave alone.
    fn config(scenario: &str) -> Config {
        let linux = LinuxConfig::new(APP_ID);
        Config::new().linux(match scenario {
            PORTAL | NAMED | STALE_ENTRY => {
                linux.app_notifications(APP_NAME, mark("icon.png").to_string_lossy())
            }
            _ => linux,
        })
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

    /// The UI a headless process started, which records what it drained and the token it got.
    fn started_ui(received: &mpsc::Receiver<Event>) {
        let seen = next_message(received, QUIET).map_or_else(
            || "nothing".to_owned(),
            |message| {
                format!(
                    "{} started_app={}",
                    String::from_utf8_lossy(&message.payload),
                    message.started_app
                )
            },
        );
        let token = std::env::var("XDG_ACTIVATION_TOKEN").unwrap_or_default();
        std::fs::write(mark("ui"), format!("{seen} token={token}")).expect("the ui mark");
    }

    /// The app side, run in a child process on the private bus.
    /// The render's pushes render, refuse, or unwind in the app before the background handler
    /// runs, and nothing else arrives on the channel.
    fn child_rendering(received: &mpsc::Receiver<Event>) {
        std::fs::write(mark("ready"), b"").expect("the ready mark");
        while !mark("done").exists() {
            std::thread::sleep(Duration::from_millis(50));
        }
        for kind in ["show", "no-alert", "panic"] {
            let seen = next_message(received, BOUND).expect("the push reaches the channel");
            assert_eq!(seen.payload, kind.as_bytes().to_vec());
            assert!(
                !seen.started_app,
                "a push to a running process started nothing"
            );
        }
        assert!(
            next_message(received, QUIET).is_none(),
            "a fourth message arrived"
        );
    }

    fn child(scenario: &str) {
        let activated = std::env::var_os("PUSHUPS_DBUS_ACTIVATED").is_some();
        // A process the bus started for a tap stays inside `install`, and starts the UI process.
        pushups::install(config(scenario)).expect("install connects to the private bus");
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
                    .icon("dialog-information")
                    .data("tap-data")
                    .show()
                    .expect("the portal shows the notification");
                Notification::new("Untagged")
                    .icon(mark("icon.png").to_string_lossy())
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
            PORTAL_CLOSED | LAUNCHED if !mark("installed").exists() => {
                std::fs::write(mark("installed"), b"").expect("the installed mark");
            }
            PORTAL_CLOSED | LAUNCHED => started_ui(&received),
            NO_BACKEND => {
                Notification::new("Unbacked")
                    .show()
                    .expect("Notify shows the notification");
                std::fs::write(mark("shown"), b"").expect("the shown mark");
            }
            FALLBACK => fallback(&received),
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
            NAMED => {
                Notification::new("Named")
                    .show()
                    .expect("the fallback shows the notification");
                Notification::new("Own")
                    .icon("/usr/share/own.png")
                    .show()
                    .expect("the fallback shows the notification with its own icon");
                std::fs::write(mark("shown"), b"").expect("the shown mark");
            }
            STALE_ENTRY => {
                Notification::new("Stale")
                    .tag("stale-1")
                    .show()
                    .expect("the portal shows the notification");
                std::fs::write(mark("shown"), b"").expect("the shown mark");
            }
            RENDERING => child_rendering(&received),
            HUNG => {
                let started = Instant::now();
                let shown = Notification::new("Held").show();
                let took = started.elapsed();
                assert!(
                    matches!(shown, Err(Error::Platform(_))),
                    "show to a server that never answers gave {shown:?}"
                );
                assert!(
                    took >= SHOW_BOUND && took < SHOW_BOUND + Duration::from_secs(3),
                    "show returned after {took:?}"
                );
            }
            other => panic!("unknown scenario {other}"),
        }
    }

    fn fallback(received: &mpsc::Receiver<Event>) {
        Notification::new("Title")
            .body("Body")
            .tag("tag-1")
            .icon(mark("icon.png").to_string_lossy())
            .data("fb-data")
            .show()
            .expect("Notify shows the notification");
        Notification::new("Again")
            .tag("tag-1")
            .data("fb-data")
            .show()
            .expect("Notify replaces the notification");
        Notification::new("Closed")
            .tag("tag-2")
            .data("closed-data")
            .show()
            .expect("Notify shows the notification the server closes");
        std::fs::write(mark("shown"), b"").expect("the shown mark");
        let tap = next_message(received, BOUND).expect("the tap arrives");
        assert!(
            mark("real").exists(),
            "a forged ActionInvoked or another action key was delivered"
        );
        assert_eq!(tap.payload, b"fb-data");
        assert!(!tap.started_app);
        assert!(
            next_message(received, QUIET * 2).is_none(),
            "a click on a closed notification was delivered"
        );
    }
}
