//! A stand-in `org.freedesktop.Notifications` server for the test bed, which has no desktop. It
//! numbers each `Notify` from 1, appends it to `notifier.log` in `$PROBE_OUT`, and on
//! `rs.pushups.TestNotifier.TriggerAction(id, action)` emits `ActionInvoked` as a click would.
//! It runs until killed.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedValue;

const PATH: &str = "/org/freedesktop/Notifications";

fn log(line: &str) {
    let dir =
        std::env::var_os("PROBE_OUT").map_or_else(|| PathBuf::from("/tmp/probe"), PathBuf::from);
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("notifier.log"))
    {
        let _ = writeln!(file, "{line}");
    }
}

struct Notifications(AtomicU32);

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
        let id = if replaces_id == 0 {
            self.0.fetch_add(1, Ordering::SeqCst) + 1
        } else {
            replaces_id
        };
        log(&format!(
            "notify id={id} app={app_name} summary={summary} actions={actions:?} body={body}"
        ));
        id
    }

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;
}

struct Trigger;

#[zbus::interface(name = "rs.pushups.TestNotifier")]
impl Trigger {
    /// Emits `ActionInvoked` from the server's own connection, as a click on the notification would.
    async fn trigger_action(
        &self,
        id: u32,
        action: String,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<()> {
        let notifications = server
            .interface::<_, Notifications>(PATH)
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        Notifications::action_invoked(notifications.signal_emitter(), id, &action)
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        log(&format!("triggered id={id} action={action}"));
        Ok(())
    }
}

fn main() {
    let _connection = zbus::blocking::connection::Builder::session()
        .and_then(|builder| builder.name("org.freedesktop.Notifications"))
        .and_then(|builder| builder.serve_at(PATH, Notifications(AtomicU32::new(0))))
        .and_then(|builder| builder.serve_at(PATH, Trigger))
        .and_then(zbus::blocking::connection::Builder::build)
        .expect("the notification server owns its name");
    log("serving");
    loop {
        std::thread::park();
    }
}
