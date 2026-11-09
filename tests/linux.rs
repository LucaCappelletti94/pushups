//! The Linux backend against a fake UnifiedPush distributor on a private session bus.
//!
//! `main` starts `dbus-daemon` and the distributor, then runs this binary again as the app, with
//! `PUSHUPS_TEST_CHILD` naming the scenario, since `install` binds the process to one bus.

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
    use std::os::fd::AsRawFd as _;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[cfg(feature = "encryption")]
    use p256::elliptic_curve::point::AffineCoordinates;
    #[cfg(feature = "encryption")]
    use pushups::Token;
    use pushups::{Config, Error, Event, LinuxConfig, Permission, UnifiedPushConfig};
    use zbus::zvariant::{OwnedValue, Value};

    const CHILD: &str = "PUSHUPS_TEST_CHILD";
    /// The file the child writes once `register` returned, holding the milliseconds it took.
    const MARK: &str = "PUSHUPS_TEST_MARK";
    /// The directory the child writes its scenario marks in.
    const MARKS: &str = "PUSHUPS_TEST_MARKS";
    const HOLD: &str = "hold";
    const REFUSE: &str = "refuse";
    const UNREGISTER: &str = "unregister";
    const NO_DISTRIBUTOR: &str = "no-distributor";
    const DATA_IS_FILE: &str = "data-is-file";
    const NAME_TAKEN: &str = "name-taken";
    /// A stale process keeps the queue's lock, so `install` gives up on it after its bound.
    const QUEUE_HELD: &str = "queue-held";
    /// The stale process that keeps the queue's lock.
    const QUEUE_OWNER: &str = "queue-owner";
    /// A headless process asks for a name a UI process holds without allowing the replacement.
    const NAME_TAKEN_HEADLESS: &str = "name-taken-headless";
    /// The process that holds the name for [`NAME_TAKEN_HEADLESS`].
    const NAME_OWNER: &str = "name-owner";
    /// A push that names a token the app does not know.
    const FOREIGN_TOKEN: &str = "foreign-token";
    /// The system ships the activation file, which `install` must leave alone.
    const SYSTEM_SERVICE: &str = "system-service";
    const REREGISTER: &str = "reregister";
    #[cfg(feature = "encryption")]
    const ROTATE: &str = "rotate";
    #[cfg(feature = "encryption")]
    const ROTATE_FIRST: &str = "rotate-first";
    #[cfg(feature = "encryption")]
    const ROTATE_WRITE_FAILED: &str = "rotate-write-failed";
    /// A second push of an identical payload delivers nothing.
    #[cfg(feature = "encryption")]
    const DUPLICATE: &str = "duplicate";
    const APP_ID: &str = "rs.pushups.LinuxTest";
    /// The endpoint the rotation scenarios register with, carried by the token events.
    #[cfg(feature = "encryption")]
    const ENDPOINT: &str = "https://push.example/rotate";
    /// How long a scenario may wait on the child, generous for a loaded runner.
    const BOUND: Duration = Duration::from_secs(20);

    pub fn main() {
        if let Ok(scenario) = std::env::var(CHILD) {
            child(&scenario);
            return;
        }
        let scenarios = [
            HOLD,
            REFUSE,
            UNREGISTER,
            NO_DISTRIBUTOR,
            DATA_IS_FILE,
            NAME_TAKEN,
            QUEUE_HELD,
            NAME_TAKEN_HEADLESS,
            FOREIGN_TOKEN,
            SYSTEM_SERVICE,
            REREGISTER,
        ];
        #[cfg(feature = "encryption")]
        let scenarios =
            scenarios
                .into_iter()
                .chain([ROTATE, ROTATE_FIRST, ROTATE_WRITE_FAILED, DUPLICATE]);
        for scenario in scenarios {
            run(scenario);
            println!("linux scenario {scenario}: ok");
        }
    }

    /// The fake distributor, which holds every `Register` call, refuses it, or accepts it and
    /// then unregisters the app.
    struct Distributor {
        scenario: &'static str,
        calls: Arc<AtomicUsize>,
    }

    #[zbus::interface(name = "org.unifiedpush.Distributor2")]
    impl Distributor {
        async fn register(
            &self,
            args: HashMap<String, OwnedValue>,
            #[zbus(connection)] connection: &zbus::Connection,
        ) -> HashMap<String, OwnedValue> {
            if args
                .get("service")
                .and_then(|value| <&str>::try_from(value).ok())
                == Some(APP_ID)
            {
                self.calls.fetch_add(1, Ordering::SeqCst);
            }
            match self.scenario {
                // KUnifiedPush queues the call unanswered while it believes the machine is offline.
                HOLD => std::future::pending::<()>().await,
                UNREGISTER | REREGISTER => {
                    let token = args
                        .get("token")
                        .and_then(|value| <&str>::try_from(value).ok())
                        .expect("Register carries the app's token");
                    let unregistered = HashMap::from([("token", Value::from(token))]);
                    // The app may exit as soon as it saw the event, before its reply arrives.
                    let _ = connection
                        .call_method(
                            Some(APP_ID),
                            "/org/unifiedpush/Connector",
                            Some("org.unifiedpush.Connector2"),
                            "Unregistered",
                            &(unregistered,),
                        )
                        .await;
                    return HashMap::from([(
                        "success".to_owned(),
                        owned("REGISTRATION_SUCCEEDED"),
                    )]);
                }
                #[cfg(feature = "encryption")]
                ROTATE | ROTATE_FIRST | ROTATE_WRITE_FAILED | DUPLICATE => {
                    return HashMap::from([(
                        "success".to_owned(),
                        owned("REGISTRATION_SUCCEEDED"),
                    )]);
                }
                _ => {}
            }
            HashMap::from([
                ("success".to_owned(), owned("REGISTRATION_FAILED")),
                ("reason".to_owned(), owned("VAPID_REQUIRED")),
            ])
        }
    }

    fn owned(text: &str) -> OwnedValue {
        Value::from(text)
            .try_into()
            .expect("a string converts to an owned value")
    }

    /// A child process killed when dropped, so a failed assertion leaves nothing running.
    struct Reaped(Child);

    impl Drop for Reaped {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// Starts `dbus-daemon` on a private session bus and returns it with its address.
    fn start_bus() -> (Reaped, String) {
        let mut daemon = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("dbus-daemon starts");
        let mut address = String::new();
        BufReader::new(daemon.stdout.take().expect("the daemon's stdout is piped"))
            .read_line(&mut address)
            .expect("dbus-daemon prints its address");
        (Reaped(daemon), address.trim().to_owned())
    }

    /// Polls `done` until it holds or `bound` passes.
    fn wait_until_within(mut done: impl FnMut() -> bool, bound: Duration) -> bool {
        let started = Instant::now();
        while started.elapsed() < bound {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        done()
    }

    /// Polls `done` until it holds or [`BOUND`] passes.
    fn wait_until(done: impl FnMut() -> bool) -> bool {
        wait_until_within(done, BOUND)
    }

    /// The system services directory of the [`SYSTEM_SERVICE`] scenario, holding the shipped
    /// activation file.
    fn system_service_dir(scenario: &str, data: &Path) -> Option<PathBuf> {
        (scenario == SYSTEM_SERVICE).then(|| {
            let services = data.join("sys").join("dbus-1").join("services");
            std::fs::create_dir_all(&services).expect("the system services directory is created");
            std::fs::write(
                services.join(format!("{APP_ID}.service")),
                "[D-BUS Service]\nName={APP_ID}\n",
            )
            .expect("the system activation file is written");
            data.join("sys")
        })
    }

    /// The parent side of the scenarios that run a process beside the main child.
    fn drive(scenario: &str, address: &str, data: &Path) {
        #[cfg(feature = "encryption")]
        if matches!(scenario, ROTATE | ROTATE_FIRST | ROTATE_WRITE_FAILED) {
            drive_rotation(scenario, address, data);
        }
        #[cfg(feature = "encryption")]
        if scenario == DUPLICATE {
            drive_duplicate(address, data);
        }
        if scenario == QUEUE_HELD {
            drive_queue_owner(data);
        }
        if scenario == NAME_TAKEN_HEADLESS {
            drive_name_owner(address, data);
        }
        if scenario == FOREIGN_TOKEN {
            drive_foreign_token(address, data);
        }
    }

    fn run(scenario: &'static str) {
        let (_daemon, address) = start_bus();
        let calls = Arc::new(AtomicUsize::new(0));
        let _distributor = (scenario != NO_DISTRIBUTOR).then(|| {
            zbus::blocking::connection::Builder::address(address.as_str())
                .expect("the bus address parses")
                .name("org.unifiedpush.Distributor.fake")
                .expect("the distributor name is valid")
                .serve_at(
                    "/org/unifiedpush/Distributor",
                    Distributor {
                        scenario,
                        calls: Arc::clone(&calls),
                    },
                )
                .expect("the distributor object is served")
                .build()
                .expect("the distributor connects")
        });
        let data = tempfile::tempdir().expect("a temporary data directory");
        let mark = data.path().join("register-returned");
        let marks = data.path().join("marks");
        std::fs::create_dir_all(&marks).expect("the marks directory is created");
        let data_home = if scenario == DATA_IS_FILE {
            let file = data.path().join("not-a-directory");
            std::fs::write(&file, b"").expect("the blocking file is written");
            file
        } else {
            data.path().to_owned()
        };
        let sys = system_service_dir(scenario, data.path());
        let mut command = Command::new(std::env::current_exe().expect("this binary's path"));
        command
            .env(CHILD, scenario)
            .env(MARK, &mark)
            .env(MARKS, &marks)
            .env("DBUS_SESSION_BUS_ADDRESS", &address)
            .env("XDG_DATA_HOME", &data_home)
            .env_remove("UNIFIEDPUSH_DISTRIBUTOR");
        if scenario == NAME_TAKEN_HEADLESS {
            command.env("PUSHUPS_DBUS_ACTIVATED", "1");
        }
        if let Some(sys) = &sys {
            command.env("XDG_DATA_DIRS", sys);
        }
        let mut app = Reaped(command.spawn().expect("the app process starts"));
        drive(scenario, &address, data.path());
        if matches!(scenario, HOLD | REFUSE | UNREGISTER) {
            assert!(
                wait_until(|| calls.load(Ordering::SeqCst) == 1 && mark.exists()),
                "scenario {scenario}: within {BOUND:?} the distributor got {} Register calls and register {}",
                calls.load(Ordering::SeqCst),
                if mark.exists() {
                    "returned"
                } else {
                    "had not returned"
                },
            );
            let took = std::fs::read_to_string(&mark).expect("the mark is readable");
            assert!(
                took.trim().parse::<u64>().is_ok_and(|ms| ms < 2_000),
                "scenario {scenario}: register took {took} ms"
            );
        }
        if scenario == REREGISTER {
            assert!(
                wait_until(|| calls.load(Ordering::SeqCst) == 2),
                "scenario {scenario}: within {BOUND:?} the distributor got {} Register calls",
                calls.load(Ordering::SeqCst),
            );
        }
        if scenario != HOLD {
            // The queue's claim waits its full bound before it fails.
            let bound = if scenario == QUEUE_HELD {
                Duration::from_secs(50)
            } else {
                BOUND
            };
            let mut status = None;
            assert!(
                wait_until_within(
                    || {
                        status = app.0.try_wait().expect("the app can be waited on");
                        status.is_some()
                    },
                    bound,
                ),
                "scenario {scenario}: the app still ran after {bound:?}"
            );
            assert!(
                status.is_some_and(|status| status.success()),
                "scenario {scenario}: the app failed with {status:?}"
            );
        }
    }

    /// The app side, run in the child process on the private bus.
    fn child(scenario: &str) {
        if scenario == NAME_TAKEN {
            return name_taken();
        }
        if scenario == QUEUE_HELD {
            return queue_held();
        }
        if scenario == QUEUE_OWNER {
            return queue_owner();
        }
        if scenario == NAME_OWNER {
            return name_owner();
        }
        if scenario == NAME_TAKEN_HEADLESS {
            return name_taken_headless();
        }
        if scenario == FOREIGN_TOKEN {
            return foreign_token();
        }
        if scenario == SYSTEM_SERVICE {
            return system_service();
        }
        #[cfg(feature = "encryption")]
        if scenario == DUPLICATE {
            return duplicate();
        }
        #[cfg(feature = "encryption")]
        if matches!(scenario, ROTATE | ROTATE_FIRST | ROTATE_WRITE_FAILED) {
            return rotate_child(scenario);
        }
        if scenario == REREGISTER {
            return reregister();
        }
        let permission = futures_executor::block_on(pushups::request_permission());
        assert_eq!(
            permission,
            Ok(if scenario == NO_DISTRIBUTOR {
                Permission::Denied
            } else {
                Permission::Granted
            }),
            "request_permission before install gave {permission:?}"
        );
        let installed = pushups::install(config());
        if scenario == DATA_IS_FILE {
            assert!(
                matches!(installed, Err(Error::Platform(_))),
                "install with a data directory it cannot create gave {installed:?}"
            );
            return;
        }
        installed.expect("install connects to the private bus");
        let status = futures_executor::block_on(pushups::permission_status());
        assert_eq!(
            status,
            Ok(Permission::Granted),
            "the status after install gave {status:?}"
        );
        let (events, received) = mpsc::channel();
        pushups::set_handler(move |event| {
            let _ = events.send(event);
        });
        let started = Instant::now();
        pushups::register().expect("register starts");
        let took = started.elapsed().as_millis();
        let mark = std::env::var_os(MARK).expect("the parent names the mark");
        std::fs::write(Path::new(&mark), took.to_string()).expect("the mark is written");
        let expected: &[&str] = match scenario {
            // The parent ends this process once it saw the held call and the mark.
            HOLD => {
                std::thread::sleep(BOUND * 2);
                return;
            }
            REFUSE => &["VAPID_REQUIRED", "org.unifiedpush.Distributor.fake"],
            UNREGISTER => &[],
            NO_DISTRIBUTOR => &["no UnifiedPush distributor"],
            other => panic!("unknown scenario {other}"),
        };
        let deadline = Instant::now() + BOUND;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match received.recv_timeout(left) {
                Ok(Event::RegistrationFailed(error)) => {
                    assert_eq!(
                        scenario == UNREGISTER,
                        error == Error::Unregistered,
                        "{scenario}: {error:?}"
                    );
                    let error = error.to_string();
                    for part in expected {
                        assert!(error.contains(part), "{scenario}: {error}");
                    }
                    return;
                }
                Ok(_) => {}
                Err(error) => panic!("{scenario}: no RegistrationFailed within {BOUND:?}: {error}"),
            }
        }
    }

    fn config() -> Config {
        Config::new()
            .linux(LinuxConfig::new(APP_ID))
            .unified_push(UnifiedPushConfig::new([4; 65]))
    }

    /// The distributor unregisters the app, and the handler, which the connector's `Unregistered`
    /// call delivers to, registers again from inside that call, as an app does on that reason.
    fn reregister() {
        pushups::install(config()).expect("install connects to the private bus");
        let (failures, received) = mpsc::channel();
        let retried = std::sync::atomic::AtomicBool::new(false);
        pushups::set_handler(move |event| {
            if let Event::RegistrationFailed(error) = event {
                // The flag guards only its own value, so no ordering beyond the swap is needed.
                if error == Error::Unregistered && !retried.swap(true, Ordering::Relaxed) {
                    pushups::register().expect("register starts from the handler");
                }
                let _ = failures.send(error);
            }
        });
        pushups::register().expect("register starts");
        for attempt in ["first", "second"] {
            let error = received
                .recv_timeout(BOUND)
                .unwrap_or_else(|_| panic!("no {attempt} RegistrationFailed within {BOUND:?}"));
            assert_eq!(error, Error::Unregistered, "{attempt}");
        }
    }

    /// Another connection owns the app's name and does not let it go, so `install` fails, fails
    /// again on a retry, and succeeds once the name is free.
    fn name_taken() {
        let other = zbus::blocking::Connection::session().expect("a second connection to the bus");
        let reply = other
            .request_name_with_flags(APP_ID, zbus::fdo::RequestNameFlags::DoNotQueue.into())
            .expect("the other connection asks for the name");
        assert_eq!(reply, zbus::fdo::RequestNameReply::PrimaryOwner);
        let message = format!("another process of {APP_ID} owns its bus name");
        for attempt in ["first", "second"] {
            let installed = pushups::install(config());
            let Err(Error::Platform(reason)) = installed else {
                panic!(
                    "the {attempt} install while another connection owns the name gave {installed:?}"
                );
            };
            assert_eq!(reason, message, "the {attempt} install");
        }
        other
            .release_name(APP_ID)
            .expect("the other connection releases the name");
        pushups::install(config()).expect("install owns the name once it is free");
        let owner = zbus::blocking::fdo::DBusProxy::new(&other)
            .expect("a bus proxy")
            .get_name_owner(APP_ID.try_into().expect("the app id is a bus name"))
            .expect("the name has an owner");
        assert_ne!(
            owner.as_str(),
            other
                .unique_name()
                .expect("the other connection's name")
                .as_str(),
            "the other connection still owns the name"
        );
    }

    /// A stale process that keeps the queue's lock, the way a hung one would until it is killed.
    /// It takes the lock itself, since the queue is one process's by that file.
    fn queue_owner() {
        let home = std::env::var_os("XDG_DATA_HOME").expect("the parent names the data home");
        let dir = Path::new(&home).join(APP_ID).join("pushups");
        std::fs::create_dir_all(&dir).expect("the queue directory is created");
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join("queue.lock"))
            .expect("the lock file opens");
        // SAFETY: the descriptor stays open across the sleep that ends this process.
        assert_eq!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0,
            "the stale process takes the queue's lock"
        );
        let marks = std::env::var_os(MARKS).expect("the parent names the marks");
        std::fs::write(Path::new(&marks).join("queue-held"), b"").expect("the mark is written");
        std::thread::sleep(Duration::from_secs(60));
    }

    /// `install` asks for the queue the stale process still holds, waits its full bound, and
    /// fails naming the app.
    fn queue_held() {
        let marks = std::env::var_os(MARKS).expect("the parent names the marks");
        assert!(
            wait_until(|| Path::new(&marks).join("queue-held").exists()),
            "the stale process never held the queue"
        );
        let installed = pushups::install(config());
        let Err(Error::Platform(message)) = installed else {
            panic!("install while a stale process holds the queue gave {installed:?}");
        };
        assert!(
            message.contains(&format!(
                "another process of {APP_ID} still holds its queue"
            )),
            "{message}"
        );
    }

    /// Holds the app's bus name without allowing the replacement a headless process asks for.
    fn name_owner() {
        pushups::install(config()).expect("install owns the name");
        let marks = std::env::var_os(MARKS).expect("the parent names the marks");
        std::fs::write(Path::new(&marks).join("name-held"), b"").expect("the mark is written");
        std::thread::sleep(Duration::from_secs(60));
    }

    /// A headless process asks for a name a UI process holds, and the bus queues the request
    /// rather than grants it.
    fn name_taken_headless() {
        let marks = std::env::var_os(MARKS).expect("the parent names the marks");
        assert!(
            wait_until(|| Path::new(&marks).join("name-held").exists()),
            "the name owner never held the name"
        );
        let installed = pushups::install(config());
        let Err(Error::Platform(message)) = installed else {
            panic!("install behind a name holder gave {installed:?}");
        };
        assert_eq!(
            message,
            format!("another process of {APP_ID} owns its bus name")
        );
    }

    /// A push that names a token the app does not know reaches the connector, echoes its id,
    /// and delivers nothing.
    fn foreign_token() {
        pushups::install(config()).expect("install connects to the private bus");
        let (events, received) = mpsc::channel();
        pushups::set_handler(move |event| {
            let _ = events.send(event);
        });
        pushups::register().expect("register starts");
        let error = received
            .recv_timeout(BOUND)
            .unwrap_or_else(|_| panic!("no RegistrationFailed within {BOUND:?}"));
        match error {
            Event::RegistrationFailed(_) => {}
            other => panic!("no RegistrationFailed, got {other:?}"),
        }
        let marks = std::env::var_os(MARKS).expect("the parent names the marks");
        std::fs::write(Path::new(&marks).join("foreign-ready"), b"").expect("the mark is written");
        assert!(
            received.recv_timeout(Duration::from_secs(4)).is_err(),
            "a push with a token the app does not know was delivered"
        );
    }

    /// The system ships the app's activation file, so `install` writes no file for the user.
    fn system_service() {
        pushups::install(config()).expect("install connects to the private bus");
        let home = std::env::var_os("XDG_DATA_HOME").expect("the parent names the data home");
        let user = Path::new(&home)
            .join("dbus-1")
            .join("services")
            .join(format!("{APP_ID}.service"));
        assert!(
            !user.exists(),
            "install wrote a user activation file over the system's"
        );
    }

    /// The second push of an identical payload delivers nothing.
    #[cfg(feature = "encryption")]
    fn duplicate() {
        pushups::install(config()).expect("install connects to the private bus");
        let (events, received) = mpsc::channel();
        pushups::set_handler(move |event| {
            let _ = events.send(event);
        });
        pushups::register().expect("register starts");
        let marks = std::env::var_os(MARKS).expect("the parent names the marks");
        std::fs::write(Path::new(&marks).join("dup-ready"), b"").expect("the mark is written");
        let event = received
            .recv_timeout(BOUND)
            .unwrap_or_else(|_| panic!("no first push within {BOUND:?}"));
        let Event::Message(message) = event else {
            panic!("no first push, got {event:?}");
        };
        assert_eq!(message.payload, b"once");
        assert!(
            received.recv_timeout(Duration::from_secs(4)).is_err(),
            "a second push of the same payload was delivered"
        );
    }

    /// The app's state directory under `home`, holding the queue, keys and token.
    #[cfg(feature = "encryption")]
    fn state_dir(home: &Path) -> PathBuf {
        home.join(APP_ID).join("pushups")
    }

    /// The public key the 48-byte `keys` file's scalar signs with, the 65-byte `p256dh` form.
    #[cfg(feature = "encryption")]
    fn public_key(keys: &[u8]) -> [u8; 65] {
        let scalar: [u8; 32] = keys[..32].try_into().expect("the scalar is 32 bytes");
        let scalar = p256::SecretKey::from_bytes(&scalar.into()).expect("the scalar is in range");
        let public = scalar.public_key();
        let point = public.as_affine();
        let mut p256dh = [0u8; 65];
        p256dh[0] = 0x04;
        for (slot, coordinate) in p256dh[1..]
            .iter_mut()
            .zip(point.x().iter().chain(point.y().iter()))
        {
            *slot = *coordinate;
        }
        p256dh
    }

    /// The 16-byte auth secret of the `keys` file contents.
    #[cfg(feature = "encryption")]
    fn auth_secret(keys: &[u8]) -> [u8; 16] {
        keys[32..48]
            .try_into()
            .expect("the auth secret is 16 bytes")
    }

    /// Seals `plaintext` for the `keys` file contents, one RFC 8291 body.
    #[cfg(feature = "encryption")]
    fn seal(keys: &[u8], plaintext: &[u8]) -> Vec<u8> {
        let token = pushups::Token::WebPush {
            endpoint: ENDPOINT.to_owned(),
            p256dh: public_key(keys),
            auth: auth_secret(keys),
            expires: None,
        };
        pushups_sender::encrypt(&token, plaintext)
            .expect("the seal encrypts")
            .into_bytes()
    }

    /// The parent side of the queue scenarios. It starts the stale process that keeps the
    /// queue's lock, and the main child's `install` then gives up on the queue after its bound.
    fn drive_queue_owner(data: &Path) {
        let marks = data.join("marks");
        let holder = Reaped(
            Command::new(std::env::current_exe().expect("this binary's path"))
                .env(CHILD, QUEUE_OWNER)
                .env(MARKS, &marks)
                .env("XDG_DATA_HOME", data)
                .spawn()
                .expect("the stale process starts"),
        );
        assert!(
            wait_until(|| marks.join("queue-held").exists()),
            "the stale process never held the queue"
        );
        // The holder must outlive the main child's 30-second claim, so it is not reaped here.
        std::mem::forget(holder);
    }

    /// The parent side of the headless name scenario. It starts the process that holds the name
    /// without allowing the replacement.
    fn drive_name_owner(address: &str, data: &Path) {
        let marks = data.join("marks");
        let owner = Reaped(
            Command::new(std::env::current_exe().expect("this binary's path"))
                .env(CHILD, NAME_OWNER)
                .env(MARKS, &marks)
                .env("DBUS_SESSION_BUS_ADDRESS", address)
                .env("XDG_DATA_HOME", data)
                .spawn()
                .expect("the name holder starts"),
        );
        assert!(
            wait_until(|| marks.join("name-held").exists()),
            "the name holder never held the name"
        );
        std::mem::forget(owner);
    }

    /// The parent side of the foreign token scenario. It waits for the child to be ready, pushes
    /// with a token the child does not know, and checks the reply.
    fn drive_foreign_token(address: &str, data: &Path) {
        let marks = data.join("marks");
        assert!(
            wait_until(|| marks.join("foreign-ready").exists()),
            "the child never became ready"
        );
        let connection = zbus::blocking::connection::Builder::address(address)
            .expect("the bus address parses")
            .build()
            .expect("a connection for the push");
        let args = HashMap::from([
            ("token", Value::from("00000000-0000-4000-8000-000000000000")),
            ("message", Value::from(vec![0u8; 8])),
            ("id", Value::from("foreign-1")),
        ]);
        let reply = connection
            .call_method(
                Some(APP_ID),
                "/org/unifiedpush/Connector",
                Some("org.unifiedpush.Connector2"),
                "Message",
                &args,
            )
            .expect("the foreign push reaches the connector");
        let reply: HashMap<String, OwnedValue> =
            reply.body().deserialize().expect("the reply deserializes");
        assert_eq!(
            reply.len(),
            1,
            "the reply to a foreign push carries only the id"
        );
        let id = reply
            .get("id")
            .and_then(|id| <&str>::try_from(id).ok())
            .unwrap_or("-");
        assert_eq!(id, "foreign-1", "the reply echoes the push's id");
    }

    /// The parent side of the duplicate scenario. It seals one payload and pushes it twice.
    #[cfg(feature = "encryption")]
    fn drive_duplicate(address: &str, data: &Path) {
        let marks = data.join("marks");
        assert!(
            wait_until(|| marks.join("dup-ready").exists()),
            "the child never became ready"
        );
        let dir = state_dir(data);
        let token = std::fs::read_to_string(dir.join("token")).expect("the token file is readable");
        let keys = std::fs::read(dir.join("keys")).expect("the keys file is readable");
        let sealed = seal(&keys, b"once");
        let connection = zbus::blocking::connection::Builder::address(address)
            .expect("the bus address parses")
            .build()
            .expect("a connection for the pushes");
        for id in ["dup-1", "dup-2"] {
            let args = HashMap::from([
                ("token", Value::from(token.as_str())),
                ("message", Value::from(sealed.as_slice())),
                ("id", Value::from(id)),
            ]);
            connection
                .call_method(
                    Some(APP_ID),
                    "/org/unifiedpush/Connector",
                    Some("org.unifiedpush.Connector2"),
                    "Message",
                    &args,
                )
                .expect("the duplicate push reaches the connector");
        }
    }

    /// Reports the endpoint to the connector, so the app reports the token with its current keys.
    #[cfg(feature = "encryption")]
    fn new_endpoint_call(connection: &zbus::blocking::Connection, token: &str) {
        let args = HashMap::from([
            ("endpoint", Value::from(ENDPOINT)),
            ("token", Value::from(token)),
        ]);
        connection
            .call_method(
                Some(APP_ID),
                "/org/unifiedpush/Connector",
                Some("org.unifiedpush.Connector2"),
                "NewEndpoint",
                &args,
            )
            .expect("the endpoint reaches the connector");
    }

    /// Sends one sealed push to the connector, returning the bus's answer or its refusal.
    #[cfg(feature = "encryption")]
    fn push_call(
        connection: &zbus::blocking::Connection,
        token: &str,
        sealed: &[u8],
    ) -> Result<(), zbus::Error> {
        let args = HashMap::from([
            ("token", Value::from(token)),
            ("message", Value::from(sealed)),
            ("id", Value::from("rotate-1")),
        ]);
        connection
            .call_method(
                Some(APP_ID),
                "/org/unifiedpush/Connector",
                Some("org.unifiedpush.Connector2"),
                "Message",
                &args,
            )
            .map(|_| ())
    }

    /// Drives a rotation scenario. The parent reports the endpoint, the child rotates the keys,
    /// and the pushes sealed with `pushups-sender` are refused for the old keys and arrive for
    /// the new ones.
    #[cfg(feature = "encryption")]
    fn drive_rotation(scenario: &str, address: &str, data: &Path) {
        let dir = state_dir(data);
        let marks = data.join("marks");
        assert!(
            wait_until(|| marks.join("ready").exists()),
            "{scenario}: the child never became ready"
        );
        let token = std::fs::read_to_string(dir.join("token")).expect("the token file is readable");
        let before = std::fs::read(dir.join("keys")).expect("the keys file is readable");
        assert_eq!(before.len(), 48, "{scenario}: the keys file");
        let connection = zbus::blocking::connection::Builder::address(address)
            .expect("the bus address parses")
            .build()
            .expect("a connection for the pushes");
        if scenario == ROTATE_WRITE_FAILED {
            // The keys path holds a directory, so the rotation's write fails.
            std::fs::remove_file(dir.join("keys")).expect("the keys file is removed");
            std::fs::create_dir(dir.join("keys")).expect("the keys path is a directory");
            std::fs::write(marks.join("rotate"), b"").expect("the rotate mark is written");
            assert!(
                wait_until(|| marks.join("rotated-failed").exists()),
                "{scenario}: the child never reported the failed rotate"
            );
            let sealed = seal(&before, b"old-keys-still-work");
            push_call(&connection, &token, sealed.as_slice())
                .expect("the old-keys push reaches the connector");
            assert!(
                wait_until(|| marks.join("delivered").exists()),
                "{scenario}: the old-keys push never arrived"
            );
            return;
        }
        if scenario == ROTATE {
            // The endpoint arrives before the rotation, so the token the child sees carries the
            // old keys.
            new_endpoint_call(&connection, &token);
        } else {
            // The rotation happens before any endpoint, so it reports no token.
            std::fs::write(marks.join("rotate"), b"").expect("the rotate mark is written");
        }
        assert!(
            wait_until(|| marks.join("rotated").exists()),
            "{scenario}: the child never rotated"
        );
        if scenario == ROTATE_FIRST {
            // The rotation preceded the first endpoint, so it reported no token, and the
            // endpoint's token carries the new keys.
            new_endpoint_call(&connection, &token);
        }
        let after = std::fs::read(dir.join("keys")).expect("the keys file is readable");
        assert_eq!(after.len(), 48, "{scenario}: the keys file");
        assert_ne!(
            &before[..],
            &after[..],
            "{scenario}: the keys did not change"
        );
        let stale = seal(&before, b"stale");
        let error = push_call(&connection, &token, stale.as_slice())
            .expect_err("{scenario}: a push sealed for the old keys still decrypts");
        assert!(
            error
                .to_string()
                .contains("the record failed authentication"),
            "{scenario}: the stale push was refused with {error}"
        );
        let fresh = seal(&after, b"rotated");
        push_call(&connection, &token, fresh.as_slice())
            .expect("the new-keys push reaches the connector");
        assert!(
            wait_until(|| marks.join("delivered").exists()),
            "{scenario}: the new-keys push never arrived"
        );
    }

    /// The app side of the rotation scenarios, run in the child process on the private bus.
    #[cfg(feature = "encryption")]
    fn rotate_child(scenario: &str) {
        pushups::install(config()).expect("install connects to the private bus");
        let (events, received) = mpsc::channel();
        pushups::set_handler(move |event| {
            let _ = events.send(event);
        });
        pushups::register().expect("register starts");
        let home = std::env::var_os("XDG_DATA_HOME").expect("the parent names the data home");
        let dir = state_dir(Path::new(&home));
        let marks = std::env::var_os(MARKS).expect("the parent names the marks");
        let marks = Path::new(&marks);
        std::fs::write(marks.join("ready"), b"").expect("the ready mark is written");
        let first = if scenario == ROTATE {
            // The endpoint's token arrives first, so the child rotates after seeing it carry the
            // old keys.
            let event = received
                .recv_timeout(BOUND)
                .unwrap_or_else(|_| panic!("{scenario}: no first token within {BOUND:?}"));
            match event {
                Event::Token(Token::WebPush {
                    endpoint,
                    p256dh,
                    auth,
                    ..
                }) => {
                    assert_eq!(endpoint, ENDPOINT, "{scenario}: the endpoint");
                    let (stored_p256dh, stored_auth) = stored_keys(&dir);
                    assert_eq!(p256dh, stored_p256dh, "{scenario}: the first token");
                    assert_eq!(auth, stored_auth, "{scenario}: the first token");
                    pushups::rotate_keys().expect("the keys rotate");
                    std::fs::write(marks.join("rotated"), b"").expect("the mark is written");
                    Some((p256dh, auth))
                }
                other => panic!("{scenario}: no first token, got {other:?}"),
            }
        } else {
            // Before the first endpoint the rotation reports no token.
            assert!(
                wait_until(|| marks.join("rotate").exists()),
                "{scenario}: the parent never triggered the rotation"
            );
            let rotated = pushups::rotate_keys();
            if scenario == ROTATE_WRITE_FAILED {
                assert!(
                    matches!(rotated, Err(Error::Platform(_))),
                    "the failing rotate gave {rotated:?}"
                );
                std::fs::write(marks.join("rotated-failed"), b"").expect("the mark is written");
                wait_for_message(scenario, &received, marks, b"old-keys-still-work");
                return;
            }
            rotated.expect("the keys rotate");
            std::fs::write(marks.join("rotated"), b"").expect("the mark is written");
            None
        };
        let event = received
            .recv_timeout(BOUND)
            .unwrap_or_else(|_| panic!("{scenario}: no token after the rotation within {BOUND:?}"));
        match event {
            Event::Token(Token::WebPush {
                endpoint,
                p256dh,
                auth,
                ..
            }) => {
                assert_eq!(endpoint, ENDPOINT, "{scenario}: the endpoint");
                let (stored_p256dh, stored_auth) = stored_keys(&dir);
                assert_eq!(
                    p256dh, stored_p256dh,
                    "{scenario}: the token after the rotation"
                );
                assert_eq!(
                    auth, stored_auth,
                    "{scenario}: the token after the rotation"
                );
                if let Some((old_p256dh, old_auth)) = first {
                    assert_ne!(
                        p256dh, old_p256dh,
                        "{scenario}: the token keeps the old keys"
                    );
                    assert_ne!(auth, old_auth, "{scenario}: the token keeps the old auth");
                }
            }
            other => panic!("{scenario}: no token after the rotation, got {other:?}"),
        }
        wait_for_message(scenario, &received, marks, b"rotated");
    }

    /// The child's ground truth, the public key and auth secret of the current `keys` file.
    #[cfg(feature = "encryption")]
    fn stored_keys(dir: &Path) -> ([u8; 65], [u8; 16]) {
        let keys = std::fs::read(dir.join("keys")).expect("the keys file is readable");
        assert_eq!(keys.len(), 48, "the keys file");
        (public_key(&keys), auth_secret(&keys))
    }

    /// Waits for the rotation scenario's delivered push, checks its payload, and marks it.
    #[cfg(feature = "encryption")]
    fn wait_for_message(
        scenario: &str,
        received: &mpsc::Receiver<pushups::Event>,
        marks: &Path,
        payload: &[u8],
    ) {
        let event = received
            .recv_timeout(BOUND)
            .unwrap_or_else(|_| panic!("{scenario}: no push within {BOUND:?}"));
        match event {
            Event::Message(message) => {
                assert_eq!(message.payload, payload, "{scenario}: the payload");
                std::fs::write(marks.join("delivered"), b"").expect("the mark is written");
                // The process exits once this returns, so give the connector's executor the
                // reply to the parent's `Message` call before it goes.
                std::thread::sleep(Duration::from_millis(500));
            }
            other => panic!("{scenario}: no delivered push, got {other:?}"),
        }
    }
}
