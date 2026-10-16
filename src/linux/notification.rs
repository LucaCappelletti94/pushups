//! Desktop notifications on Linux, and the taps on them.
//!
//! A notification goes through the xdg desktop portal when its host Registry names the app, so
//! a tap on a closed app starts it through D-Bus activation and arrives as `ActivateAction`. Where
//! the portal or its Registry is missing, it goes through `org.freedesktop.Notifications`, whose
//! `ActionInvoked` reaches only the process that showed the notification.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Write as _};
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{Fd, OwnedValue, Value};

use super::{ACTIVATED_ENV, Call, RUNTIME, Runtime};
use crate::dispatch::DISPATCHER;
use crate::queue::Entry;
use crate::{Error, Message, Notification};

/// The action a tap runs, exported as `app.pushups-tap` to the portal.
pub(super) const TAP_ACTION: &str = "pushups-tap";
const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const NOTIFICATIONS_NAME: &str = "org.freedesktop.Notifications";
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";
/// How long `show` waits for the notification service.
const SHOW_BOUND: Duration = Duration::from_secs(5);
/// How long a `show` waits for the portal's Registry before it uses the fallback, so a portal
/// that is slow to start leaves time for `Notify` within [`SHOW_BOUND`].
const REGISTER_BOUND: Duration = Duration::from_secs(2);

/// Which service shows this process's notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Service {
    /// Not chosen yet, or the portal restarted and needs the app id again.
    Unknown,
    /// The Registry has not answered yet, so notifications use the fallback meanwhile.
    Registering,
    Portal,
    Fallback,
}

/// The notification state of this process.
pub(super) struct Notifier {
    service: Service,
    shown: u64,
    /// The fallback's notification id of each tag, so a later one replaces it.
    tags: HashMap<String, u32>,
    /// The fallback's `data` of each notification, for its `ActionInvoked`.
    data: HashMap<u32, String>,
    listening: bool,
}

impl Notifier {
    /// The notifier of a process that found its desktop entry in place, which may try the
    /// portal, or of one that had to write it, which uses the fallback, since the desktop shell
    /// has not indexed the new entry yet and drops a portal notification for an app it does not know.
    pub(super) fn new(entry_in_place: bool) -> Self {
        Self {
            service: if entry_in_place {
                Service::Unknown
            } else {
                Service::Fallback
            },
            shown: 0,
            tags: HashMap::new(),
            data: HashMap::new(),
            listening: false,
        }
    }
}

/// The connection notifications go through. It is not the one that owns the app's name, whose
/// executor may be busy in the call that runs the handler which calls `show`.
static CONNECTION: OnceLock<Connection> = OnceLock::new();

/// Shows `notification`, waiting at most [`SHOW_BOUND`] for the service.
pub(crate) fn show(notification: &Notification) -> Result<(), Error> {
    let runtime = RUNTIME
        .get()
        .filter(|runtime| runtime.owns_name.load(std::sync::atomic::Ordering::Acquire))
        .ok_or(Error::NotConfigured)?;
    let notification = notification.clone();
    let (sender, reply) = mpsc::channel();
    thread::Builder::new()
        .name("pushups show".to_owned())
        .spawn(move || {
            let _ = sender.send(show_now(runtime, &notification));
        })
        .map_err(Error::platform)?;
    reply.recv_timeout(SHOW_BOUND).unwrap_or_else(|_| {
        Err(Error::Platform(format!(
            "the notification service did not answer within {} s",
            SHOW_BOUND.as_secs()
        )))
    })
}

fn show_now(runtime: &'static Runtime, notification: &Notification) -> Result<(), Error> {
    let connection = if let Some(connection) = CONNECTION.get() {
        connection
    } else {
        let connection = Connection::session().map_err(Error::platform)?;
        CONNECTION.get_or_init(|| connection)
    };
    let unchosen = {
        let mut notifier = runtime.notifier.lock();
        let unchosen = notifier.service == Service::Unknown;
        if unchosen {
            notifier.service = Service::Registering;
        }
        unchosen
    };
    if unchosen {
        choose_service(connection, runtime);
    }
    let mut notifier = runtime.notifier.lock();
    notifier.shown += 1;
    if notifier.service == Service::Portal {
        let id = portal_id(notification, std::process::id(), notifier.shown);
        let mut vardict = portal_notification(notification);
        if let Some(icon) = notification.icon.as_deref().and_then(portal_icon) {
            vardict.insert("icon", icon);
        }
        match connection.call_method(
            Some(PORTAL_NAME),
            PORTAL_PATH,
            Some("org.freedesktop.portal.Notification"),
            "AddNotification",
            &(id, vardict),
        ) {
            Ok(_) => return Ok(()),
            // A portal without a notification backend leaves the fallback.
            Err(zbus::Error::MethodError(name, _, _))
                if name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod"
                    || name.as_str() == "org.freedesktop.DBus.Error.UnknownInterface" =>
            {
                notifier.service = Service::Fallback;
            }
            Err(error) => return Err(Error::platform(error)),
        }
    }
    notify(connection, runtime, &mut notifier, notification)
}

/// Registers the app id with the portal on a thread of its own, and waits at most
/// [`REGISTER_BOUND`] for the answer, which sets the service whenever it comes.
fn choose_service(connection: &'static Connection, runtime: &'static Runtime) {
    let (answered, answer) = mpsc::channel();
    let registering = thread::Builder::new()
        .name("pushups register".to_owned())
        .spawn(move || {
            let service = if register(connection, &runtime.app_id) {
                watch_portal(connection, runtime);
                Service::Portal
            } else {
                Service::Fallback
            };
            runtime.notifier.lock().service = service;
            let _ = answered.send(());
        });
    match registering {
        Ok(_) => {
            let _ = answer.recv_timeout(REGISTER_BOUND);
        }
        Err(_) => runtime.notifier.lock().service = Service::Fallback,
    }
}

/// Associates this connection with the app id, which the portal needs before it shows anything.
fn register(connection: &Connection, app_id: &str) -> bool {
    connection
        .call_method(
            Some(PORTAL_NAME),
            PORTAL_PATH,
            Some("org.freedesktop.host.portal.Registry"),
            "Register",
            &(app_id, HashMap::<&str, Value<'_>>::new()),
        )
        .is_ok()
}

/// Forgets the portal's registration when the portal restarts, so the next `show` registers again.
fn watch_portal(connection: &Connection, runtime: &'static Runtime) {
    let Ok(proxy) = DBusProxy::new(connection) else {
        return;
    };
    let Ok(changes) = proxy.receive_name_owner_changed_with_args(&[(0, PORTAL_NAME)]) else {
        return;
    };
    let _ = thread::Builder::new()
        .name("pushups portal".to_owned())
        .spawn(move || {
            for _ in changes {
                runtime.notifier.lock().service = Service::Unknown;
            }
        });
}

/// The portal's id for `notification`, its tag, else one of its own.
fn portal_id(notification: &Notification, pid: u32, shown: u64) -> String {
    notification
        .tag
        .clone()
        .unwrap_or_else(|| format!("pushups-{pid}-{shown}"))
}

/// The portal's notification vardict, without the icon.
fn portal_notification(notification: &Notification) -> HashMap<&'static str, Value<'_>> {
    let mut vardict = HashMap::from([("title", Value::from(notification.title.as_str()))]);
    if let Some(body) = &notification.body {
        vardict.insert("body", Value::from(body.as_str()));
    }
    if let Some(data) = &notification.data {
        vardict.insert("default-action", Value::from(format!("app.{TAP_ACTION}")));
        vardict.insert("default-action-target", Value::from(data.as_str()));
    }
    vardict
}

/// The portal's serialized icon, a sealed memory file for a local image or a themed name.
fn portal_icon(icon: &str) -> Option<Value<'static>> {
    if !icon.contains('/') {
        return Some(Value::from(("themed", Value::from(vec![icon.to_owned()]))));
    }
    let bytes = fs::read(icon).ok()?;
    let fd = sealed_memfd(&bytes).ok()?;
    Some(Value::from(("file-descriptor", Value::from(Fd::from(fd)))))
}

/// A memory file holding `bytes`, sealed, as the portal accepts an icon file.
fn sealed_memfd(bytes: &[u8]) -> io::Result<OwnedFd> {
    // SAFETY: the name is NUL-terminated, and the flags are valid for `memfd_create`.
    let raw = unsafe {
        libc::memfd_create(
            c"pushups-icon".as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `memfd_create` returned a new descriptor that nothing else owns.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let mut file = File::from(fd);
    file.write_all(bytes)?;
    let seals = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_WRITE | libc::F_SEAL_SEAL;
    // SAFETY: the descriptor is open and was created with `MFD_ALLOW_SEALING`.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_ADD_SEALS, seals) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(OwnedFd::from(file))
}

/// The fallback `Notify` arguments for `notification`, replacing `replaces`.
fn notify_actions(notification: &Notification) -> Vec<&'static str> {
    if notification.data.is_some() {
        vec!["default", "View"]
    } else {
        Vec::new()
    }
}

fn notify(
    connection: &Connection,
    runtime: &'static Runtime,
    notifier: &mut Notifier,
    notification: &Notification,
) -> Result<(), Error> {
    if !notifier.listening {
        listen(connection, runtime)?;
        notifier.listening = true;
    }
    let replaces = notification
        .tag
        .as_ref()
        .and_then(|tag| notifier.tags.get(tag))
        .copied()
        .unwrap_or(0);
    let local_icon = notification
        .icon
        .as_deref()
        .filter(|icon| icon.starts_with('/'))
        .unwrap_or("");
    let mut hints = HashMap::from([
        ("x-canonical-app-name", Value::from(runtime.app_id.as_str())),
        ("desktop-entry", Value::from(runtime.app_id.as_str())),
    ]);
    if !local_icon.is_empty() {
        hints.insert("image-path", Value::from(local_icon));
    }
    let reply = connection
        .call_method(
            Some(NOTIFICATIONS_NAME),
            NOTIFICATIONS_PATH,
            Some(NOTIFICATIONS_NAME),
            "Notify",
            &(
                runtime.app_id.as_str(),
                replaces,
                local_icon,
                notification.title.as_str(),
                notification.body.as_deref().unwrap_or(""),
                notify_actions(notification),
                hints,
                0_i32,
            ),
        )
        .map_err(Error::platform)?;
    let id: u32 = reply.body().deserialize().map_err(Error::platform)?;
    if let Some(tag) = &notification.tag {
        notifier.tags.insert(tag.clone(), id);
    }
    match &notification.data {
        Some(data) => notifier.data.insert(id, data.clone()),
        None => notifier.data.remove(&id),
    };
    Ok(())
}

/// Listens for the server's `ActionInvoked` and `NotificationClosed`. The proxy matches only
/// signals from the owner of the server's name, so no neighbour on the bus forges a tap.
fn listen(connection: &Connection, runtime: &'static Runtime) -> Result<(), Error> {
    let proxy = Proxy::new(
        connection,
        NOTIFICATIONS_NAME,
        NOTIFICATIONS_PATH,
        NOTIFICATIONS_NAME,
    )
    .map_err(Error::platform)?;
    let invoked = proxy
        .receive_signal("ActionInvoked")
        .map_err(Error::platform)?;
    let closed = proxy
        .receive_signal("NotificationClosed")
        .map_err(Error::platform)?;
    thread::Builder::new()
        .name("pushups taps".to_owned())
        .spawn(move || {
            for signal in invoked {
                let Ok((id, action)) = signal.body().deserialize::<(u32, String)>() else {
                    continue;
                };
                if action != "default" {
                    continue;
                }
                let data = runtime.notifier.lock().data.get(&id).cloned();
                if let Some(data) = data {
                    tap(runtime, data.into_bytes(), &HashMap::new());
                }
            }
        })
        .map_err(Error::platform)?;
    thread::Builder::new()
        .name("pushups closes".to_owned())
        .spawn(move || {
            for signal in closed {
                let Ok((id, _reason)) = signal.body().deserialize::<(u32, u32)>() else {
                    continue;
                };
                let mut notifier = runtime.notifier.lock();
                notifier.data.remove(&id);
                notifier.tags.retain(|_, tagged| *tagged != id);
            }
        })
        .map_err(Error::platform)?;
    Ok(())
}

/// A tap on one of the app's notifications. It reaches the handler once per payload, and a
/// headless process keeps it for the UI it then starts.
fn tap(runtime: &'static Runtime, payload: Vec<u8>, platform: &HashMap<String, OwnedValue>) {
    {
        let state = runtime.state.lock();
        // A delivered set that cannot be read or written lets the tap through, since a repeat beats a loss.
        if state.delivered.first_delivery(&payload).unwrap_or(true) {
            state.inbox.receive(Entry::Message(Message {
                payload,
                started_app: runtime.headless,
            }));
        }
    }
    DISPATCHER.flush();
    if runtime.headless {
        launch_ui(runtime, platform);
    }
}

/// Starts the app's UI from a headless process, once, with the activation token that lets its
/// window take focus. The UI takes the bus name over and drains what this process kept.
fn launch_ui(runtime: &'static Runtime, platform: &HashMap<String, OwnedValue>) {
    static LAUNCHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if runtime.activity.lock().replaced || LAUNCHED.swap(true, std::sync::atomic::Ordering::AcqRel)
    {
        return;
    }
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let mut command = Command::new(executable);
    command
        .args(std::env::args_os().skip(1))
        .env_remove(ACTIVATED_ENV)
        .stdin(Stdio::null())
        .process_group(0);
    for (key, variable) in [
        ("activation-token", "XDG_ACTIVATION_TOKEN"),
        ("desktop-startup-id", "DESKTOP_STARTUP_ID"),
    ] {
        if let Some(token) = platform
            .get(key)
            .and_then(|value| <&str>::try_from(value).ok())
        {
            command.env(variable, token);
        }
    }
    // A UI that cannot start leaves the tap in the queue for the next one the user opens.
    let _ = command.spawn();
}

/// The `org.freedesktop.Application` object the portal activates for a tap.
pub(super) struct Application;

#[expect(
    clippy::unused_self,
    clippy::needless_pass_by_value,
    reason = "zbus calls these on the served object and hands them owned arguments"
)]
#[zbus::interface(name = "org.freedesktop.Application")]
impl Application {
    /// A launcher opened the app.
    fn activate(&self, platform_data: HashMap<String, OwnedValue>) -> zbus::fdo::Result<()> {
        let call = Call::begin()?;
        if call.0.headless {
            launch_ui(call.0, &platform_data);
        }
        Ok(())
    }

    /// A launcher opened files with the app.
    fn open(
        &self,
        uris: Vec<String>,
        platform_data: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<()> {
        drop(uris);
        self.activate(platform_data)
    }

    /// The portal ran one of the app's actions, a tap on a notification for `pushups-tap`.
    fn activate_action(
        &self,
        action_name: String,
        parameter: Vec<OwnedValue>,
        platform_data: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<()> {
        let call = Call::begin()?;
        if action_name != TAP_ACTION {
            return Err(zbus::fdo::Error::InvalidArgs(format!(
                "unknown action {action_name}"
            )));
        }
        let data = parameter
            .first()
            .and_then(|value| <&str>::try_from(value).ok())
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("no tap data".to_owned()))?;
        tap(call.0, data.as_bytes().to_vec(), &platform_data);
        Ok(())
    }
}

/// The object path of the app's `org.freedesktop.Application`, from its app id.
pub(super) fn object_path(app_id: &str) -> String {
    format!("/{}", app_id.replace('.', "/").replace('-', "_"))
}

/// The desktop entry the portal needs to accept the app id, hidden from launchers.
pub(super) fn desktop_file(app_id: &str, executable: &Path) -> String {
    let mut quoted = String::new();
    for character in executable.to_string_lossy().chars() {
        if matches!(character, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    format!(
        "[Desktop Entry]\nType=Application\nName={app_id}\nExec=\"{quoted}\"\nNoDisplay=true\nDBusActivatable=true\n"
    )
}

/// Whether a desktop entry for the app exists in the user's or the system's data directories.
pub(super) fn has_desktop_file(
    app_id: &str,
    data_home: Option<&Path>,
    data_dirs: Option<OsString>,
) -> bool {
    let dirs = data_dirs
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| OsString::from("/usr/local/share:/usr/share"));
    data_home
        .map(Path::to_path_buf)
        .into_iter()
        .chain(std::env::split_paths(&dirs))
        .any(|dir| {
            dir.join("applications")
                .join(format!("{app_id}.desktop"))
                .exists()
        })
}

/// Writes the hidden desktop entry when no other names the app, so a packaged app's own wins.
/// True when an entry was in place already.
pub(super) fn write_desktop_file(app_id: &str, data_home: &Path) -> io::Result<bool> {
    if has_desktop_file(app_id, Some(data_home), std::env::var_os("XDG_DATA_DIRS")) {
        return Ok(true);
    }
    let dir: PathBuf = data_home.join("applications");
    fs::create_dir_all(&dir)?;
    let executable = std::env::current_exe()?;
    fs::write(
        dir.join(format!("{app_id}.desktop")),
        desktop_file(app_id, &executable),
    )?;
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_object_path_follows_the_app_id() {
        assert_eq!(object_path("org.example.My-App"), "/org/example/My_App");
    }

    #[test]
    fn the_desktop_entry_is_hidden_and_quotes_its_executable() {
        assert_eq!(
            desktop_file("org.example.App", Path::new("/opt/my \"app\"/bin/$app")),
            "[Desktop Entry]\nType=Application\nName=org.example.App\nExec=\"/opt/my \\\"app\\\"/bin/\\$app\"\nNoDisplay=true\nDBusActivatable=true\n"
        );
    }

    #[test]
    fn a_desktop_entry_in_the_system_data_dirs_is_found() {
        let home = tempfile::tempdir().expect("a data home");
        let system = tempfile::tempdir().expect("a system data dir");
        let dirs = std::env::join_paths([system.path()]).expect("one data dir");
        assert!(!has_desktop_file(
            "org.example.App",
            Some(home.path()),
            Some(dirs.clone())
        ));
        fs::create_dir_all(system.path().join("applications")).expect("applications");
        fs::write(
            system.path().join("applications/org.example.App.desktop"),
            "",
        )
        .expect("a packaged entry");
        assert!(has_desktop_file(
            "org.example.App",
            Some(home.path()),
            Some(dirs)
        ));
    }

    #[test]
    fn a_tagged_notification_keeps_its_tag_as_the_portal_id() {
        let tagged = Notification::new("t").tag("chat");
        assert_eq!(portal_id(&tagged, 9, 3), "chat");
        assert_eq!(portal_id(&Notification::new("t"), 9, 3), "pushups-9-3");
    }

    #[test]
    fn the_portal_vardict_carries_the_tap_only_with_data() {
        let plain = Notification::new("Title");
        let vardict = portal_notification(&plain);
        assert_eq!(vardict.len(), 1);
        let tapped = Notification::new("Title").body("Body").data("d");
        let vardict = portal_notification(&tapped);
        assert_eq!(
            vardict.get("default-action"),
            Some(&Value::from("app.pushups-tap"))
        );
        assert_eq!(
            vardict.get("default-action-target"),
            Some(&Value::from("d"))
        );
        assert_eq!(vardict.get("body"), Some(&Value::from("Body")));
        assert_eq!(notify_actions(&plain), [""; 0]);
        assert_eq!(notify_actions(&tapped), ["default", "View"]);
    }
}
