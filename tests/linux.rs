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

    use pushups::{Config, Event, LinuxConfig, UnifiedPushConfig};
    use zbus::zvariant::{OwnedValue, Value};

    const CHILD: &str = "PUSHUPS_TEST_CHILD";
    /// The file the child writes once `register` returned, holding the milliseconds it took.
    const MARK: &str = "PUSHUPS_TEST_MARK";
    const HOLD: &str = "hold";
    const REFUSE: &str = "refuse";
    const APP_ID: &str = "rs.pushups.LinuxTest";
    /// How long a scenario may wait on the child, generous for a loaded runner.
    const BOUND: Duration = Duration::from_secs(20);

    pub fn main() {
        if let Ok(scenario) = std::env::var(CHILD) {
            child(&scenario);
            return;
        }
        for scenario in [HOLD, REFUSE] {
            run(scenario);
            println!("linux scenario {scenario}: ok");
        }
    }

    /// The fake distributor, which holds every `Register` call or refuses it.
    struct Distributor {
        hold: bool,
        calls: Arc<AtomicUsize>,
    }

    #[zbus::interface(name = "org.unifiedpush.Distributor2")]
    impl Distributor {
        async fn register(&self, args: HashMap<String, OwnedValue>) -> HashMap<String, OwnedValue> {
            if args
                .get("service")
                .and_then(|value| <&str>::try_from(value).ok())
                == Some(APP_ID)
            {
                self.calls.fetch_add(1, Ordering::SeqCst);
            }
            if self.hold {
                // KUnifiedPush queues the call unanswered while it believes the machine is offline.
                std::future::pending::<()>().await;
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

    fn run(scenario: &str) {
        let (_daemon, address) = start_bus();
        let calls = Arc::new(AtomicUsize::new(0));
        let _distributor = zbus::blocking::connection::Builder::address(address.as_str())
            .expect("the bus address parses")
            .name("org.unifiedpush.Distributor.fake")
            .expect("the distributor name is valid")
            .serve_at(
                "/org/unifiedpush/Distributor",
                Distributor {
                    hold: scenario == HOLD,
                    calls: Arc::clone(&calls),
                },
            )
            .expect("the distributor object is served")
            .build()
            .expect("the distributor connects");
        let data = tempfile::tempdir().expect("a temporary data directory");
        let mark = data.path().join("register-returned");
        let mut app = Reaped(
            Command::new(std::env::current_exe().expect("this binary's path"))
                .env(CHILD, scenario)
                .env(MARK, &mark)
                .env("DBUS_SESSION_BUS_ADDRESS", &address)
                .env("XDG_DATA_HOME", data.path())
                .env_remove("UNIFIEDPUSH_DISTRIBUTOR")
                .spawn()
                .expect("the app process starts"),
        );
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
        if scenario == REFUSE {
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
        pushups::install(
            Config::new()
                .linux(LinuxConfig::new(APP_ID))
                .unified_push(UnifiedPushConfig::new([4; 65])),
        )
        .expect("install connects to the private bus");
        let (events, received) = mpsc::channel();
        pushups::set_handler(move |event| {
            let _ = events.send(event);
        });
        let started = Instant::now();
        pushups::register().expect("register starts");
        let took = started.elapsed().as_millis();
        let mark = std::env::var_os(MARK).expect("the parent names the mark");
        std::fs::write(Path::new(&mark), took.to_string()).expect("the mark is written");
        match scenario {
            // The parent ends this process once it saw the held call and the mark.
            HOLD => std::thread::sleep(BOUND * 2),
            REFUSE => {
                let deadline = Instant::now() + BOUND;
                loop {
                    let left = deadline.saturating_duration_since(Instant::now());
                    match received.recv_timeout(left) {
                        Ok(Event::RegistrationFailed(error)) => {
                            let error = error.to_string();
                            assert!(error.contains("VAPID_REQUIRED"), "{error}");
                            assert!(
                                error.contains("org.unifiedpush.Distributor.fake"),
                                "{error}"
                            );
                            return;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            panic!("no RegistrationFailed within {BOUND:?} of a refusal: {error}")
                        }
                    }
                }
            }
            other => panic!("unknown scenario {other}"),
        }
    }
}
