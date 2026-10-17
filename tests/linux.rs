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
    use std::path::Path;
    use std::process::{Child, Command, Stdio};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use pushups::{Config, Error, Event, LinuxConfig, Permission, UnifiedPushConfig};
    use zbus::zvariant::{OwnedValue, Value};

    const CHILD: &str = "PUSHUPS_TEST_CHILD";
    /// The file the child writes once `register` returned, holding the milliseconds it took.
    const MARK: &str = "PUSHUPS_TEST_MARK";
    const HOLD: &str = "hold";
    const REFUSE: &str = "refuse";
    const UNREGISTER: &str = "unregister";
    const NO_DISTRIBUTOR: &str = "no-distributor";
    const DATA_IS_FILE: &str = "data-is-file";
    const NAME_TAKEN: &str = "name-taken";
    const REREGISTER: &str = "reregister";
    const APP_ID: &str = "rs.pushups.LinuxTest";
    /// How long a scenario may wait on the child, generous for a loaded runner.
    const BOUND: Duration = Duration::from_secs(20);

    pub fn main() {
        if let Ok(scenario) = std::env::var(CHILD) {
            child(&scenario);
            return;
        }
        for scenario in [
            HOLD,
            REFUSE,
            UNREGISTER,
            NO_DISTRIBUTOR,
            DATA_IS_FILE,
            NAME_TAKEN,
            REREGISTER,
        ] {
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
        let data_home = if scenario == DATA_IS_FILE {
            let file = data.path().join("not-a-directory");
            std::fs::write(&file, b"").expect("the blocking file is written");
            file
        } else {
            data.path().to_owned()
        };
        let mut app = Reaped(
            Command::new(std::env::current_exe().expect("this binary's path"))
                .env(CHILD, scenario)
                .env(MARK, &mark)
                .env("DBUS_SESSION_BUS_ADDRESS", &address)
                .env("XDG_DATA_HOME", &data_home)
                .env_remove("UNIFIEDPUSH_DISTRIBUTOR")
                .spawn()
                .expect("the app process starts"),
        );
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
            let mut status = None;
            assert!(
                wait_until(|| {
                    status = app.0.try_wait().expect("the app can be waited on");
                    status.is_some()
                }),
                "scenario {scenario}: the app still ran after {BOUND:?}"
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
            name_taken();
            return;
        }
        if scenario == REREGISTER {
            reregister();
            return;
        }
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
        for attempt in ["first", "second"] {
            let installed = pushups::install(config());
            assert!(
                matches!(installed, Err(Error::Platform(_))),
                "the {attempt} install while another connection owns the name gave {installed:?}"
            );
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
}
