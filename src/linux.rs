//! The UnifiedPush backend for Linux, a connector on the D-Bus session bus (spec `DBUS_0.3.0`).
//!
//! `install` serves `org.unifiedpush.Connector2` under the app's id and writes the activation file
//! that lets the session bus start the app for a push. A process started that way stays inside
//! `install`, headless, as the plan's Linux delivery table lays out.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs::{self, DirBuilder, OpenOptions};
use std::future::{Ready, ready};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use std::{env, process, thread};

use parking_lot::Mutex;
use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::{Connection, connection};
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::zvariant::{OwnedValue, Value};

use crate::dispatch::DISPATCHER;
use crate::queue::{Entry, QueueFile};
use crate::session::{Route, Session};
use crate::token::base64url;
use crate::webpush_crypto::Keys;
use crate::{Config, Context, Error, Message, Notification, Permission};

const CONNECTOR_PATH: &str = "/org/unifiedpush/Connector";
const DISTRIBUTOR_PATH: &str = "/org/unifiedpush/Distributor";
const DISTRIBUTOR_INTERFACE: &str = "org.unifiedpush.Distributor2";
const DISTRIBUTOR_PREFIX: &str = "org.unifiedpush.Distributor.";
const DISTRIBUTOR_ENV: &str = "UNIFIEDPUSH_DISTRIBUTOR";
/// Set by the activation file's `Exec`, so a process knows the bus started it for a push.
const ACTIVATED_ENV: &str = "PUSHUPS_DBUS_ACTIVATED";
/// How long a headless process waits for another call before it exits.
const IDLE: Duration = Duration::from_secs(10);
/// How often a headless process checks whether it may exit.
const IDLE_POLL: Duration = Duration::from_millis(200);

/// What `install` sets up, once per process.
struct Runtime {
    connection: Connection,
    app_id: String,
    vapid: Option<String>,
    state: Mutex<State>,
    activity: Mutex<Activity>,
}

/// The routing state, the queue and the registration, behind one lock so a drain and a
/// concurrent push keep their order.
struct State {
    session: Session,
    queue: QueueFile,
    keys: Keys,
    token: String,
    /// The next message started the app, true in a headless process until its first message.
    started_app: bool,
}

/// When a headless process may exit.
struct Activity {
    calls: usize,
    last: Instant,
    replaced: bool,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

fn platform(error: impl std::fmt::Display) -> Error {
    Error::Platform(error.to_string())
}

pub(crate) fn install(config: Config) -> Result<(), Error> {
    let linux = config.linux.ok_or(Error::NotConfigured)?;
    if RUNTIME.get().is_some() {
        return Ok(());
    }
    let headless = env::var_os(ACTIVATED_ENV).is_some();
    // The bus comes first, so a process without one leaves no file behind.
    let connection = connection::Builder::session()
        .and_then(|builder| builder.serve_at(CONNECTOR_PATH, Connector))
        .and_then(connection::Builder::build)
        .map_err(platform)?;
    let dir = data_dir(
        env::var_os("XDG_DATA_HOME"),
        env::var_os("HOME"),
        &linux.app_id,
    )
    .ok_or_else(|| Error::Platform("neither XDG_DATA_HOME nor HOME is set".to_owned()))?;
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .map_err(platform)?;
    let keys = load_keys(&dir.join("keys")).map_err(platform)?;
    let token = load_token(&dir.join("token")).map_err(platform)?;
    write_activation_file(&linux.app_id).map_err(platform)?;
    let session = if headless {
        Session::Headless
    } else {
        Session::Headless.on_session(true)
    };
    let runtime = Runtime {
        connection,
        app_id: linux.app_id,
        vapid: config
            .unified_push
            .map(|unified_push| base64url(&unified_push.vapid_public_key)),
        state: Mutex::new(State {
            session,
            queue: QueueFile::new(dir.join("queue")),
            keys,
            token,
            started_app: headless,
        }),
        activity: Mutex::new(Activity {
            calls: 0,
            last: Instant::now(),
            replaced: false,
        }),
    };
    // A concurrent `install` that won the race serves the process, and this one adds nothing.
    let runtime = match RUNTIME.set(runtime) {
        Ok(()) => RUNTIME.get(),
        Err(_) => return Ok(()),
    }
    .ok_or_else(|| Error::Platform("the runtime vanished".to_owned()))?;

    // The object is served before the name is taken, so a call the bus held for an activation
    // finds it.
    let flags = if headless {
        RequestNameFlags::AllowReplacement.into()
    } else {
        RequestNameFlags::ReplaceExisting | RequestNameFlags::DoNotQueue
    };
    let reply = runtime
        .connection
        .request_name_with_flags(runtime.app_id.as_str(), flags)
        .map_err(platform)?;
    if !matches!(
        reply,
        RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner
    ) {
        return Err(Error::Platform(format!(
            "another process of {} owns its bus name",
            runtime.app_id
        )));
    }
    if headless {
        serve_headless(runtime);
    }
    Ok(())
}

/// Serves the calls a push brought, then exits once idle or replaced by a UI process.
fn serve_headless(runtime: &'static Runtime) -> ! {
    if let Ok(proxy) = DBusProxy::new(&runtime.connection) {
        if let Ok(lost) = proxy.receive_name_lost() {
            thread::spawn(move || {
                if lost.into_iter().next().is_some() {
                    runtime.activity.lock().replaced = true;
                }
            });
        }
    }
    loop {
        {
            let activity = runtime.activity.lock();
            if activity.calls == 0 && (activity.replaced || activity.last.elapsed() >= IDLE) {
                // Taken so that no call is between persisting and returning.
                let _state = runtime.state.lock();
                process::exit(0);
            }
        }
        thread::sleep(IDLE_POLL);
    }
}

pub(crate) fn register() -> Result<(), Error> {
    let runtime = RUNTIME.get().ok_or(Error::NotConfigured)?;
    let distributor = match find_distributor(&runtime.connection) {
        Ok(distributor) => distributor,
        Err(reason) => {
            DISPATCHER.emit(Entry::RegistrationFailed(reason).into_event());
            return Ok(());
        }
    };
    // KUnifiedPush holds the reply while it believes the machine is offline, so the call waits on its own thread.
    thread::Builder::new()
        .name("pushups register".to_owned())
        .spawn(move || {
            if let Err(reason) = send_register(runtime, &distributor) {
                DISPATCHER.emit(Entry::RegistrationFailed(reason).into_event());
            }
        })
        .map(drop)
        .map_err(platform)
}

/// Sends `Register` and waits for the answer, with the failure as `RegistrationFailed` describes it.
fn send_register(runtime: &Runtime, distributor: &str) -> Result<(), String> {
    let token = runtime.state.lock().token.clone();
    let mut args: HashMap<&str, Value<'_>> = HashMap::new();
    args.insert("service", Value::from(runtime.app_id.as_str()));
    args.insert("token", Value::from(token.as_str()));
    if let Some(vapid) = &runtime.vapid {
        args.insert("vapid", Value::from(vapid.as_str()));
    }
    let failed = |error: zbus::Error| format!("registering with {distributor} failed: {error}");
    let reply = runtime
        .connection
        .call_method(
            Some(distributor),
            DISTRIBUTOR_PATH,
            Some(DISTRIBUTOR_INTERFACE),
            "Register",
            &(args,),
        )
        .map_err(failed)?;
    let reply: HashMap<String, OwnedValue> = reply.body().deserialize().map_err(failed)?;
    let succeeded = reply
        .get("success")
        .and_then(|value| <&str>::try_from(value).ok())
        == Some("REGISTRATION_SUCCEEDED");
    if succeeded {
        return Ok(());
    }
    let reason = reply
        .get("reason")
        .and_then(|value| <&str>::try_from(value).ok())
        .unwrap_or("no reason given");
    Err(format!("{distributor} refused the registration: {reason}"))
}

pub(crate) fn request_permission() -> Ready<Result<Permission, Error>> {
    let permission = match Connection::session() {
        Ok(connection) if find_distributor(&connection).is_ok() => Permission::Granted,
        _ => Permission::Denied,
    };
    ready(Ok(permission))
}

/// Binds the UI session to the handler just set, and hands it the queue if the session was
/// waiting for one.
pub(crate) fn handler_set() {
    let Some(runtime) = RUNTIME.get() else {
        return;
    };
    {
        let mut state = runtime.state.lock();
        let (bound, drain) = state.session.on_handler_set();
        if drain {
            // A queue that cannot be read stays on disk and the session stays unbound, so the
            // next handler tries again.
            if let Ok(entries) = state.queue.take_all() {
                state.session = bound;
                for entry in entries {
                    DISPATCHER.enqueue(entry.into_event());
                }
            }
        } else {
            state.session = bound;
        }
    }
    DISPATCHER.flush();
}

pub(crate) fn in_service_worker() -> bool {
    false
}

pub(crate) fn serve_service_worker<H, F>(_handler: H) -> Result<(), Error>
where
    H: Fn(Message) -> F + 'static,
    F: Future<Output = Notification> + 'static,
{
    Err(Error::Unsupported)
}

/// Persists or emits an event under the state lock, by the session's route.
fn receive(state: &State, entry: Entry) {
    match state.session.route() {
        Route::Emit => DISPATCHER.enqueue(entry.into_event()),
        // A queue that cannot be written still leaves the event in memory, where a handler of
        // this process gets it.
        Route::Persist => {
            if state.queue.append(&entry).is_err() {
                DISPATCHER.enqueue(entry.into_event());
            }
        }
    }
}

/// Counts a call from the distributor as activity for as long as it lives.
struct Call(&'static Runtime);

impl Call {
    fn begin() -> zbus::fdo::Result<Self> {
        let runtime = RUNTIME
            .get()
            .ok_or_else(|| zbus::fdo::Error::Failed("the connector is not installed".to_owned()))?;
        runtime.activity.lock().calls += 1;
        Ok(Self(runtime))
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        let mut activity = self.0.activity.lock();
        activity.calls -= 1;
        activity.last = Instant::now();
    }
}

/// The `org.unifiedpush.Connector2` object the distributor calls.
struct Connector;

#[expect(
    clippy::unused_self,
    clippy::needless_pass_by_value,
    reason = "zbus calls these on the served object and hands them owned arguments"
)]
#[zbus::interface(name = "org.unifiedpush.Connector2")]
impl Connector {
    /// A push, still encrypted as the app's server sent it.
    fn message(
        &self,
        mut args: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<HashMap<String, OwnedValue>> {
        let call = Call::begin()?;
        let mut reply = HashMap::new();
        if let Some(id) = args.remove("id") {
            reply.insert("id".to_owned(), id);
        }
        let body = args
            .remove("message")
            .and_then(|message| Vec::<u8>::try_from(message).ok())
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("no message bytes".to_owned()))?;
        let message = {
            let mut state = call.0.state.lock();
            if !known_token(&state, &args) {
                return Ok(reply);
            }
            let payload = state
                .keys
                .decrypt(&body)
                .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
            let message = Message {
                payload,
                started_app: state.started_app,
            };
            state.started_app = false;
            receive(&state, Entry::Message(message.clone()));
            message
        };
        DISPATCHER.flush();
        run_background_handler(message);
        Ok(reply)
    }

    /// The endpoint the app's server sends to, new or changed.
    fn new_endpoint(
        &self,
        mut args: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<HashMap<String, OwnedValue>> {
        let call = Call::begin()?;
        let endpoint = args
            .remove("endpoint")
            .and_then(|endpoint| String::try_from(endpoint).ok())
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("no endpoint".to_owned()))?;
        {
            let state = call.0.state.lock();
            if known_token(&state, &args) {
                let entry = Entry::WebPushToken {
                    endpoint,
                    p256dh: state.keys.public_key(),
                    auth: state.keys.auth(),
                };
                receive(&state, entry);
            }
        }
        DISPATCHER.flush();
        Ok(HashMap::new())
    }

    /// The distributor dropped the registration, so the app must register again.
    fn unregistered(
        &self,
        args: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<HashMap<String, OwnedValue>> {
        let call = Call::begin()?;
        {
            let state = call.0.state.lock();
            if known_token(&state, &args) {
                let reason = "the distributor unregistered the app".to_owned();
                receive(&state, Entry::RegistrationFailed(reason));
            }
        }
        DISPATCHER.flush();
        Ok(HashMap::new())
    }
}

/// Whether a call carries this app's token, since the spec has the connector ignore any other.
fn known_token(state: &State, args: &HashMap<String, OwnedValue>) -> bool {
    args.get("token")
        .and_then(|token| <&str>::try_from(token).ok())
        == Some(state.token.as_str())
}

/// Runs the app's `#[background_handler]`, if it has one, catching a panic.
fn run_background_handler(message: Message) {
    if let Some(handler) = crate::__private::BACKGROUND_HANDLERS.first() {
        // The panic hook already reported a panic, and the push stays queued or delivered.
        let _ = std::panic::catch_unwind(|| handler(Context { _private: () }, message));
    }
}

/// The distributor to register with, or why there is none.
fn find_distributor(connection: &Connection) -> Result<String, String> {
    let proxy = DBusProxy::new(connection).map_err(|error| error.to_string())?;
    let mut names: Vec<String> = proxy
        .list_names()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|name| name.to_string())
        .collect();
    if let Ok(activatable) = proxy.list_activatable_names() {
        names.extend(activatable.into_iter().map(|name| name.to_string()));
    }
    pick_distributor(&names, env::var(DISTRIBUTOR_ENV).ok().as_deref())
}

/// Chooses among the bus names, as the spec's "Choosing a distributor" says.
fn pick_distributor(names: &[String], chosen: Option<&str>) -> Result<String, String> {
    let mut distributors: Vec<&str> = names
        .iter()
        .map(String::as_str)
        .filter(|name| name.starts_with(DISTRIBUTOR_PREFIX))
        .collect();
    distributors.sort_unstable();
    distributors.dedup();
    if let Some(chosen) = chosen.filter(|chosen| distributors.contains(chosen)) {
        return Ok(chosen.to_owned());
    }
    match distributors.as_slice() {
        [] => Err("no UnifiedPush distributor runs on the session bus".to_owned()),
        [only] => Ok((*only).to_owned()),
        several => Err(format!(
            "several UnifiedPush distributors ({}), so set {DISTRIBUTOR_ENV} to one",
            several.join(", ")
        )),
    }
}

/// The user's data directory, an absolute `$XDG_DATA_HOME` or else `~/.local/share`.
fn data_home(xdg_data_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    xdg_data_home
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home.map(|home| PathBuf::from(home).join(".local/share")))
}

/// The directory of the queue, the keys and the token, `<data home>/<app id>/pushups`.
fn data_dir(
    xdg_data_home: Option<OsString>,
    home: Option<OsString>,
    app_id: &str,
) -> Option<PathBuf> {
    Some(data_home(xdg_data_home, home)?.join(app_id).join("pushups"))
}

/// The stored keys, or new ones stored in place of missing or unreadable ones.
fn load_keys(path: &Path) -> io::Result<Keys> {
    if let Some(keys) = fs::read(path)
        .ok()
        .and_then(|bytes| Keys::from_bytes(&bytes))
    {
        return Ok(keys);
    }
    let keys = Keys::generate().map_err(io::Error::other)?;
    write_private(path, &keys.to_bytes())?;
    Ok(keys)
}

/// The stored registration token, or a new one stored in place of a missing one.
fn load_token(path: &Path) -> io::Result<String> {
    if let Ok(token) = fs::read_to_string(path) {
        if !token.is_empty() {
            return Ok(token);
        }
    }
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    let token = uuid_v4(bytes);
    write_private(path, token.as_bytes())?;
    Ok(token)
}

/// Writes a file only its owner can read, replacing it in one step.
fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    let partial = path.with_extension("part");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&partial)?;
    file.write_all(contents)?;
    file.sync_all()?;
    fs::rename(&partial, path)
}

/// Writes the per-user activation file, unless the system ships one for the app.
fn write_activation_file(app_id: &str) -> io::Result<()> {
    let file_name = format!("{app_id}.service");
    let system_dirs = env::var_os("XDG_DATA_DIRS")
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    if env::split_paths(&system_dirs)
        .any(|dir| dir.join("dbus-1/services").join(&file_name).exists())
    {
        return Ok(());
    }
    let services = data_home(env::var_os("XDG_DATA_HOME"), env::var_os("HOME"))
        .map(|home| home.join("dbus-1/services"))
        .ok_or_else(|| io::Error::other("neither XDG_DATA_HOME nor HOME is set"))?;
    let contents = service_file(app_id, &env::current_exe()?);
    let path = services.join(file_name);
    if fs::read_to_string(&path).is_ok_and(|existing| existing == contents) {
        return Ok(());
    }
    fs::create_dir_all(&services)?;
    write_private(&path, contents.as_bytes())
}

/// The activation file, whose `Exec` marks the process as started for a push.
fn service_file(app_id: &str, executable: &Path) -> String {
    let quoted = executable.to_string_lossy().replace('\'', "'\\''");
    format!("[D-BUS Service]\nName={app_id}\nExec=/usr/bin/env {ACTIVATED_ENV}=1 '{quoted}'\n")
}

/// A version 4 UUID from 16 random bytes, the token form the spec recommends.
fn uuid_v4(mut bytes: [u8; 16]) -> String {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = bytes
        .iter()
        .fold(String::with_capacity(32), |mut hex, byte| {
            // Writing to a `String` cannot fail.
            let _ = write!(hex, "{byte:02x}");
            hex
        });
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn the_activation_file_starts_the_executable_marked_as_activated() {
        let file = service_file("org.example.App", Path::new("/opt/my app/bin/app"));
        assert_eq!(
            file,
            "[D-BUS Service]\nName=org.example.App\nExec=/usr/bin/env PUSHUPS_DBUS_ACTIVATED=1 '/opt/my app/bin/app'\n"
        );
    }

    #[test]
    fn a_quote_in_the_executable_path_survives_the_exec_line() {
        let file = service_file("org.example.App", Path::new("/home/o'neil/app"));
        assert!(
            file.ends_with("Exec=/usr/bin/env PUSHUPS_DBUS_ACTIVATED=1 '/home/o'\\''neil/app'\n")
        );
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn the_only_distributor_is_chosen() {
        let found = names(&[
            "org.freedesktop.DBus",
            "org.unifiedpush.Distributor.kde",
            ":1.4",
        ]);
        assert_eq!(
            pick_distributor(&found, None),
            Ok("org.unifiedpush.Distributor.kde".to_owned())
        );
    }

    #[test]
    fn no_distributor_is_a_failure_naming_the_cause() {
        let error = pick_distributor(&names(&["org.freedesktop.DBus"]), None).unwrap_err();
        assert!(error.contains("no UnifiedPush distributor"), "{error}");
    }

    #[test]
    fn the_environment_picks_among_several_and_an_unknown_choice_is_ignored() {
        let found = names(&[
            "org.unifiedpush.Distributor.b",
            "org.unifiedpush.Distributor.a",
        ]);
        assert_eq!(
            pick_distributor(&found, Some("org.unifiedpush.Distributor.b")),
            Ok("org.unifiedpush.Distributor.b".to_owned())
        );
        let error = pick_distributor(&found, Some("org.unifiedpush.Distributor.gone")).unwrap_err();
        assert!(error.contains("org.unifiedpush.Distributor.a"), "{error}");
        assert!(error.contains("UNIFIEDPUSH_DISTRIBUTOR"), "{error}");
    }

    #[test]
    fn a_name_seen_as_both_owned_and_activatable_counts_once() {
        let found = names(&[
            "org.unifiedpush.Distributor.kde",
            "org.unifiedpush.Distributor.kde",
        ]);
        assert_eq!(
            pick_distributor(&found, None),
            Ok("org.unifiedpush.Distributor.kde".to_owned())
        );
    }

    #[test]
    fn a_token_is_a_version_4_uuid() {
        let token = uuid_v4([0xff; 16]);
        assert_eq!(token, "ffffffff-ffff-4fff-bfff-ffffffffffff");
        let token = uuid_v4([0; 16]);
        assert_eq!(token, "00000000-0000-4000-8000-000000000000");
    }

    #[test]
    fn data_lives_under_the_xdg_data_home_or_the_home_directory() {
        assert_eq!(
            data_dir(
                Some("/data".into()),
                Some("/home/u".into()),
                "org.example.App"
            ),
            Some(PathBuf::from("/data/org.example.App/pushups"))
        );
        assert_eq!(
            data_dir(None, Some("/home/u".into()), "org.example.App"),
            Some(PathBuf::from(
                "/home/u/.local/share/org.example.App/pushups"
            ))
        );
        assert_eq!(
            data_dir(
                Some("relative".into()),
                Some("/home/u".into()),
                "org.example.App"
            ),
            Some(PathBuf::from(
                "/home/u/.local/share/org.example.App/pushups"
            ))
        );
        assert_eq!(data_dir(None, None, "org.example.App"), None);
    }
}
