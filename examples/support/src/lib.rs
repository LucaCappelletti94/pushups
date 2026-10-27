//! The instrumentation helpers every pushups example shares, so the examples log the same lines
//! with the same wall-clock times under their own prefix, and the CI scripts can match them.
//!
//! An example keeps what an app author writes, the `install`, the handler, the button that asks
//! for permission, the background handler and the worker entry, and takes the configuration from
//! the build-time keys, the log line format, the timing, the worker's notification and the desktop
//! render from this crate, calling `log` with its own prefix.

use pushups::{
    Config, Event, LinuxConfig, Message, Notification, Permission, Token, UnifiedPushConfig,
    VapidPublicKey, WebPushConfig, WnsConfig,
};

pub mod page;
pub mod text;

/// Registers again when the UnifiedPush distributor dropped the app's registration, as the
/// specification asks, and leaves every other event alone.
///
/// # Errors
///
/// The error of [`pushups::register`].
pub fn register_again(event: &Event) -> Result<(), pushups::Error> {
    match event {
        Event::RegistrationFailed(pushups::Error::Unregistered) => pushups::register(),
        _ => Ok(()),
    }
}

/// The payload's string field `name`, for a JSON payload that carries it.
#[must_use]
pub fn field(message: &Message, name: &str) -> Option<String> {
    let data: serde_json::Value = serde_json::from_slice(&message.payload).ok()?;
    data.get(name)?.as_str().map(str::to_owned)
}

/// The event as the page shows it, a push as the `body` its payload carries.
#[must_use]
pub fn shown(event: &Event) -> String {
    match event {
        Event::Token(_) => "Registered for pushes".to_owned(),
        Event::RegistrationFailed(error) => format!("Registration failed: {error}"),
        Event::Message(message) => format!(
            "Push: {}",
            field(message, "body")
                .unwrap_or_else(|| String::from_utf8_lossy(&message.payload).into_owned())
        ),
        Event::MessagesDropped => "Some pushes were dropped".to_owned(),
    }
}

/// A line of the example's list, a push shown as a card or everything else the handler saw.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Line {
    /// A push the platform delivered, shown as a card.
    Push(Push),
    /// Everything else the handler saw, shown as a compact line.
    Text(String),
}

/// The list line for `event`, received at `at`, a push card or a compact event.
#[must_use]
pub fn line(event: &Event, at: u128) -> Line {
    match event {
        Event::Message(message) => Line::Push(push_view(message, at)),
        _ => Line::Text(shown(event)),
    }
}

/// The list as one log, each line as its compact text, oldest first.
#[must_use]
pub fn log_text(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|line| match line {
            Line::Push(push) => push.line(),
            Line::Text(text) => text.clone(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The push's `seq` and its latency from the sender's `sent_at_ms` to `at`.
#[must_use]
pub fn timing(message: &Message, at: u128) -> String {
    let data: serde_json::Value = serde_json::from_slice(&message.payload).unwrap_or_default();
    let field = |name: &str| data.get(name).and_then(serde_json::Value::as_str);
    let sent = field("sent_at_ms").and_then(|sent| sent.parse::<u128>().ok());
    format!(
        "seq={} sent_at_ms={} since_sent_ms={}",
        field("seq").unwrap_or("-"),
        sent.map_or_else(|| "-".to_owned(), |sent| sent.to_string()),
        sent.map_or_else(
            || "-".to_owned(),
            |sent| at.saturating_sub(sent).to_string()
        ),
    )
}

/// A push as the example's list shows it, its `title` and `body` from the payload's fields and
/// its arrival at `at`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Push {
    /// The payload's `title`, or the payload as text.
    pub title: String,
    /// The payload's `body`, or the payload as text.
    pub body: String,
    /// The arrival as local `HH:MM:SS`.
    pub time: String,
    /// Whether the platform started the app for the push.
    pub started_app: bool,
    /// The milliseconds from the payload's `sent_at_ms` to the arrival.
    pub delay_ms: Option<u128>,
}

impl Push {
    /// The compact line the drawn windows show, `HH:MM:SS title, body, +N ms`.
    #[must_use]
    pub fn line(&self) -> String {
        match self.delay_ms {
            Some(delay) => format!("{} {}, {}, +{delay} ms", self.time, self.title, self.body),
            None => format!("{} {}, {}", self.time, self.title, self.body),
        }
    }
}

/// The push `message` the handler received at `at`, for the example's list.
#[must_use]
pub fn push_view(message: &Message, at: u128) -> Push {
    let payload = String::from_utf8_lossy(&message.payload).into_owned();
    let data: serde_json::Value = serde_json::from_slice(&message.payload).unwrap_or_default();
    let sent = data
        .get("sent_at_ms")
        .and_then(serde_json::Value::as_str)
        .and_then(|sent| sent.parse::<u128>().ok());
    Push {
        title: field(message, "title").unwrap_or_else(|| payload.clone()),
        body: field(message, "body").unwrap_or(payload),
        time: clock(at),
        started_app: message.started_app,
        delay_ms: sent.map(|sent| at.saturating_sub(sent)),
    }
}

/// The wall-clock milliseconds since 1970 as local `HH:MM:SS`, from the system's local offset.
fn clock(at: u128) -> String {
    use time::{OffsetDateTime, UtcOffset};
    let Ok(seconds) = i64::try_from(at / 1000) else {
        return "--:--:--".to_owned();
    };
    let Ok(offset) = UtcOffset::current_local_offset() else {
        return "--:--:--".to_owned();
    };
    let Ok(local) =
        OffsetDateTime::from_unix_timestamp(seconds).map(|instant| instant.to_offset(offset))
    else {
        return "--:--:--".to_owned();
    };
    format!(
        "{:02}:{:02}:{:02}",
        local.hour(),
        local.minute(),
        local.second()
    )
}

/// The token the example logs after `ui token at_ms=…`, one field per transport,
/// `subscription` for Web Push, `apns` for APNs, `fcm` for FCM and `wns` for WNS.
#[must_use]
pub fn token(token: &Token) -> String {
    if let Some(subscription) = token.web_push_subscription() {
        return format!(
            "subscription={}",
            serde_json::to_string(&subscription).expect("a subscription serialises")
        );
    }
    if let Some(hex) = token.apns_device_token() {
        return format!("apns={hex}");
    }
    match token {
        Token::Fcm { token, .. } => format!("fcm={token}"),
        Token::Wns { channel_uri, .. } => format!("wns={channel_uri}"),
        _ => format!("{token:?}"),
    }
}

/// The transport name for the page's status card, `APNs`, `FCM`, `WNS`, `Web Push` on
/// `wasm32-unknown-unknown` and `UnifiedPush` for a `WebPush` token elsewhere.
#[must_use]
pub fn transport(token: &Token) -> &'static str {
    match token {
        Token::Apns { .. } => "APNs",
        Token::Fcm { .. } => "FCM",
        Token::Wns { .. } => "WNS",
        Token::WebPush { .. } => {
            #[cfg(target_arch = "wasm32")]
            {
                "Web Push"
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                "UnifiedPush"
            }
        }
        _ => "Unknown",
    }
}

/// The token line cut to the page's width, the first and last 8 characters of [`token`] around
/// an ellipsis, or for Web Push the endpoint's host and the last 8 characters of its path.
#[must_use]
pub fn shortened(token: &Token) -> String {
    if let Token::WebPush { endpoint, .. } = token {
        return shortened_endpoint(endpoint);
    }
    let line = self::token(token);
    let chars = line.chars().count();
    if chars <= 16 {
        return line;
    }
    let head: String = line.chars().take(8).collect();
    let tail: String = line.chars().skip(chars - 8).collect();
    format!("{head}...{tail}")
}

/// The endpoint without its scheme, its path cut to the last 8 characters after the host.
fn shortened_endpoint(endpoint: &str) -> String {
    let rest = endpoint
        .split_once("://")
        .map_or(endpoint, |(_, rest)| rest);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let chars = path.chars().count();
    if chars <= 8 {
        return rest.to_owned();
    }
    let tail: String = path.chars().skip(chars - 8).collect();
    format!("{host}...{tail}")
}

/// The wall-clock milliseconds since 1970, from the standard clock on the native targets and
/// from `Date.now()` on `wasm32-unknown-unknown`, whose `SystemTime` has no clock.
#[must_use]
pub fn now_ms() -> u128 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_millis())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let ms = js_sys::Date::now();
        // Milliseconds since 1970 are positive and far below 2^53, so the conversion is exact.
        debug_assert!(ms.is_finite() && ms >= 0.0, "Date.now() is a time");
        ms as u128
    }
}

/// Logs `line` under `prefix`, to the logcat tag `pushups-example` on Android, to the console on
/// the web, and to standard output and to `pushups-example.log` in the temporary directory
/// elsewhere, which outlives a process the system launched with nothing reading its output.
///
/// On Android the tag is always `pushups-example`, the one the CI reads, and `prefix` is ignored.
pub fn log(prefix: &str, line: &str) {
    #[cfg(target_os = "android")]
    {
        use std::ffi::{CString, c_char, c_int};

        #[link(name = "log")]
        unsafe extern "C" {
            fn __android_log_write(
                priority: c_int,
                tag: *const c_char,
                text: *const c_char,
            ) -> c_int;
        }
        // The tag is the filter the CI reads, so the line carries no prefix.
        let _ = prefix;
        const INFO: c_int = 4;
        let Ok(text) = CString::new(line.replace('\0', " ")) else {
            return;
        };
        // SAFETY: both strings are NUL-terminated and stay alive for the call, and liblog only
        // reads them.
        unsafe {
            __android_log_write(INFO, c"pushups-example".as_ptr(), text.as_ptr());
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::console::log_1(&format!("{prefix} {line}").into());
    }
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    {
        use std::io::Write;
        let line = format!("{prefix} {line}");
        println!("{line}");
        let path = std::env::temp_dir().join("pushups-example.log");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            // One write per line, so lines from the handler thread and the UI never interleave.
            let _ = file.write_all(format!("{line}\n").as_bytes());
        }
    }
}

/// The push as the example renders it on Linux and Windows, its `title` and `body` or the
/// defaults, with the payload as its data. A tap on the notification it shows hands the payload
/// back as a message, and starts the app when it has exited.
#[must_use]
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub fn rendered(message: &Message) -> Notification {
    let title = field(message, "title").unwrap_or_else(|| "pushups".to_owned());
    let body = field(message, "body").unwrap_or_else(|| "A push arrived".to_owned());
    Notification::new(title)
        .body(body)
        .data(String::from_utf8_lossy(&message.payload).into_owned())
}

/// The icon of the examples' desktop notifications, the brand's 256 px PNG in this checkout, as
/// the absolute path `WnsConfig::app_notifications` and `LinuxConfig::app_notifications` take.
#[must_use]
pub fn app_icon() -> String {
    let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/brand/png/icon-macos-256.png");
    std::path::absolute(&icon)
        .unwrap_or(icon)
        .to_string_lossy()
        .into_owned()
}

/// The desktop and UnifiedPush configuration from the build-time keys, `PUSHUPS_WNS_REMOTE_ID` for
/// Windows and `PUSHUPS_VAPID_PUBLIC_KEY` (base64url) for UnifiedPush on Linux and Android.
#[must_use]
pub fn desktop_config() -> Config {
    // The bundle identifier, which also names the app on the Linux session bus.
    let mut config = Config::new().linux(
        LinuxConfig::new("rs.pushups.example").app_notifications("pushups example", app_icon()),
    );
    if let Some(remote_id) = option_env!("PUSHUPS_WNS_REMOTE_ID") {
        config = config
            .windows(WnsConfig::new(remote_id).app_notifications("pushups example", app_icon()));
    }
    if let Some(key) = vapid_public_key() {
        config = config.unified_push(UnifiedPushConfig::new(key));
    }
    config
}

/// The Web Push configuration from the build-time `PUSHUPS_VAPID_PUBLIC_KEY`, `None` without it.
///
/// `PUSHUPS_WEB_WORKER=static` registers the static worker, otherwise the Rust handler runs, from
/// `worker_path` when the example serves its own worker entry.
#[must_use]
pub fn web_push(worker_path: Option<&str>) -> Option<WebPushConfig> {
    let web = WebPushConfig::new(vapid_public_key()?);
    Some(match (option_env!("PUSHUPS_WEB_WORKER"), worker_path) {
        (Some("static"), _) => web,
        (_, Some(path)) => web.rust_handler().service_worker_path(path),
        (_, None) => web.rust_handler(),
    })
}

fn vapid_public_key() -> Option<VapidPublicKey> {
    let key = option_env!("PUSHUPS_VAPID_PUBLIC_KEY")?;
    Some(VapidPublicKey::from_base64url(key).expect("PUSHUPS_VAPID_PUBLIC_KEY is a VAPID key"))
}

/// The line an example logs for every event its handler receives.
#[must_use]
pub fn describe(event: &Event) -> String {
    let at = now_ms();
    match event {
        Event::Token(token) => format!("ui token at_ms={at} {}", self::token(token)),
        Event::RegistrationFailed(error) => format!("ui registration failed at_ms={at} {error}"),
        Event::Message(message) => format!(
            "ui message at_ms={at} started_app={} {} payload={}",
            message.started_app,
            timing(message, at),
            String::from_utf8_lossy(&message.payload)
        ),
        Event::MessagesDropped => format!("ui messages dropped at_ms={at}"),
    }
}

/// The line an example's background handler logs for `message`, entered at `entered`.
#[must_use]
pub fn handled(message: &Message, entered: u128) -> String {
    format!(
        "handler started_app={} entered_at_ms={entered} {} payload={}",
        message.started_app,
        timing(message, entered),
        String::from_utf8_lossy(&message.payload)
    )
}

/// What an example's background handler does for every push, the handler line logged through `log`.
pub fn handle_push(message: &Message, log: impl Fn(&str)) {
    log(&handled(message, now_ms()));
}

/// The answer to the permission prompt, as an example logs and shows it.
#[must_use]
pub fn permission(answer: Result<Permission, pushups::Error>) -> String {
    match answer {
        Ok(permission) => format!("{permission:?}"),
        Err(error) => format!("failed: {error}"),
    }
}

/// What an example shows for the notification permission, from a status or a prompt answer.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct PermissionUi {
    /// The state, as the example describes it on its own line.
    pub label: String,
    /// Whether a prompt can change the answer, so the example offers asking.
    pub asks: bool,
    /// Whether the platform denied the permission, so the example offers the system settings.
    pub denied: bool,
}

/// Maps the permission the platform holds or the prompt answered to what the example shows.
#[must_use]
pub fn permission_ui(status: Result<Permission, pushups::Error>) -> PermissionUi {
    match status {
        Ok(Permission::NotDetermined) => PermissionUi {
            label: "Notifications are not allowed yet".to_owned(),
            asks: true,
            denied: false,
        },
        Ok(Permission::Dismissed) => PermissionUi {
            label: "The prompt was dismissed, it can be shown again".to_owned(),
            asks: true,
            denied: false,
        },
        Ok(Permission::Granted) => PermissionUi {
            label: "Notifications are on".to_owned(),
            asks: false,
            denied: false,
        },
        Ok(Permission::Denied) => PermissionUi {
            label: "Notifications are off, turn them on in Settings".to_owned(),
            asks: false,
            denied: true,
        },
        // A platform answer `Permission` has not named yet, which the example cannot ask about.
        Ok(unknown) => PermissionUi {
            label: format!("Notifications: {unknown:?}"),
            asks: false,
            denied: false,
        },
        Err(error) => PermissionUi {
            label: format!("Notifications are unavailable: {error}"),
            asks: false,
            denied: false,
        },
    }
}

/// What an example's service worker shows for `message`, its `title` and `body` or the push's timing, after logging the timing through `log`.
pub fn worker_notification(message: &Message, log: impl Fn(&str)) -> Notification {
    let at = now_ms();
    let line = timing(message, at);
    log(&format!("worker push at_ms={at} {line}"));
    Notification::new(field(message, "title").unwrap_or_else(|| "pushups from Rust".to_owned()))
        .body(
            field(message, "body")
                .unwrap_or_else(|| format!("built in the service worker, {line}")),
        )
        .navigate("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pushups::{Event, Message, Permission, Token};

    fn fcm(token: &str) -> Token {
        Token::Fcm {
            token: token.to_owned(),
            keys: None,
        }
    }

    #[test]
    fn transport_names_each_backend() {
        assert_eq!(transport(&fcm("t")), "FCM");
        assert_eq!(
            transport(&Token::Apns {
                device_token: vec![0],
                keys: None
            }),
            "APNs"
        );
        #[cfg(not(target_arch = "wasm32"))]
        assert_eq!(transport(&web_push("https://example.com")), "UnifiedPush");
    }

    #[test]
    fn shortened_keeps_head_and_tail_around_an_ellipsis() {
        let token = fcm(&"a".repeat(40));
        let line = super::token(&token);
        let cut = shortened(&token);
        assert!(cut.starts_with(&line[..8]));
        assert!(cut.ends_with(&line[line.len() - 8..]));
        assert!(cut.contains("..."));
        assert_eq!(cut.chars().count(), 8 + 3 + 8);
    }

    #[test]
    fn shortened_leaves_a_short_line_alone() {
        let token = fcm("short");
        assert_eq!(shortened(&token), super::token(&token));
    }

    fn web_push(endpoint: &str) -> Token {
        Token::WebPush {
            endpoint: endpoint.into(),
            p256dh: [0; 65],
            auth: [0; 16],
            expires: None,
        }
    }

    #[test]
    fn shortened_names_a_web_push_endpoint_by_its_host_and_tail() {
        let token = web_push("https://fcm.googleapis.com/fcm/send/abcdefgh:12345678");
        assert_eq!(shortened(&token), "fcm.googleapis.com...12345678");
    }

    #[test]
    fn shortened_leaves_a_short_web_push_path_whole() {
        assert_eq!(shortened(&web_push("https://ntfy.sh/up1")), "ntfy.sh/up1");
    }

    #[test]
    fn push_view_reads_the_payload_fields_and_the_delay() {
        let payload = br#"{"title":"t","body":"b","sent_at_ms":"1000"}"#;
        let message = Message {
            payload: payload.to_vec(),
            started_app: true,
        };
        let view = push_view(&message, 1500);
        assert_eq!(view.title, "t");
        assert_eq!(view.body, "b");
        assert!(view.started_app);
        assert_eq!(view.delay_ms, Some(500));
        assert_eq!(view.time.chars().count(), 8);
    }

    #[test]
    fn push_view_falls_back_to_the_payload_text() {
        let message = Message {
            payload: b"plain text".to_vec(),
            started_app: false,
        };
        let view = push_view(&message, 0);
        assert_eq!(view.title, "plain text");
        assert_eq!(view.body, "plain text");
        assert!(!view.started_app);
        assert_eq!(view.delay_ms, None);
    }

    #[test]
    fn line_maps_a_push_to_a_card_and_the_rest_to_text() {
        let message = Message {
            payload: br#"{"body":"b"}"#.to_vec(),
            started_app: false,
        };
        assert!(matches!(line(&Event::Message(message), 0), Line::Push(_)));
        assert_eq!(
            line(&Event::MessagesDropped, 0),
            Line::Text("Some pushes were dropped".to_owned())
        );
    }

    #[test]
    fn permission_ui_marks_only_a_denied_permission() {
        assert!(permission_ui(Ok(Permission::Denied)).denied);
        assert!(!permission_ui(Ok(Permission::Granted)).denied);
        assert!(permission_ui(Ok(Permission::NotDetermined)).asks);
    }
}
