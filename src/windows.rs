//! The Windows backend, WNS through the Windows App SDK's `PushNotificationManager`, and
//! desktop notifications through its `AppNotificationManager`.
//!
//! An unpackaged app has no auto-initializer, so `install` runs the bootstrapper, checks
//! `IsSupported()`, puts the `PushReceived` handler in before `Register()`, and, when
//! `WnsConfig` set the display name and icon, puts the `NotificationInvoked` handler in before
//! the notification manager's `Register`. `install` then delivers the push or tap that started
//! the app, and `show` calls the manager's `Show` on a worker thread with a five-second bound.
//! `register` creates the fresh channel on a worker thread, since WNS can take up to fifteen
//! minutes to answer. Until W0 settles background activation on hardware, an unpackaged app
//! receives pushes only while it is running.

#[cfg(target_os = "windows")]
mod bindings;

#[cfg(target_os = "windows")]
mod ffi {
    windows_link::link!("kernel32.dll" "system" fn LoadLibraryW(name: *const u16) -> *mut core::ffi::c_void);
    windows_link::link!(
        "kernel32.dll"
        "system"
        fn GetProcAddress(
            module: *mut core::ffi::c_void,
            name: *const u8,
        ) -> Option<unsafe extern "system" fn() -> isize>
    );

    /// `MddBootstrapInitialize2` from `MddBootstrap.h`.
    pub type BootstrapInitialize2 = unsafe extern "system" fn(
        major_minor: u32,
        version_tag: *const u16,
        min_version: PackageVersion,
        options: u32,
    ) -> windows_core::HRESULT;

    /// `PACKAGE_VERSION`, the minimum runtime the bootstrapper accepts in its channel.
    #[repr(C)]
    pub struct PackageVersion {
        pub major: u16,
        pub minor: u16,
        pub build: u16,
        pub revision: u16,
    }
}

use std::time::{Duration, SystemTime};

#[cfg(target_os = "windows")]
use std::env;
#[cfg(target_os = "windows")]
use std::fs::DirBuilder;
#[cfg(target_os = "windows")]
use std::path::PathBuf;
#[cfg(target_os = "windows")]
use std::sync::OnceLock;
#[cfg(target_os = "windows")]
use std::sync::mpsc;
#[cfg(target_os = "windows")]
use std::thread;

#[cfg(target_os = "windows")]
use parking_lot::Mutex;
#[cfg(target_os = "windows")]
use windows::Foundation::TypedEventHandler;
#[cfg(target_os = "windows")]
use windows_core::Interface as _;

#[cfg(target_os = "windows")]
use crate::backend::Backend;
#[cfg(target_os = "windows")]
use crate::config::AppNotifications;
#[cfg(target_os = "windows")]
use crate::delivered::Delivered;
#[cfg(target_os = "windows")]
use crate::dispatch::DISPATCHER;
#[cfg(target_os = "windows")]
use crate::{Config, Error, Event, Message, Notification, Permission, Token};

/// The 100-nanosecond ticks from 1601 to the Unix epoch, the base of `Windows.Foundation.DateTime`.
const EPOCH_1601_TO_1970: i64 = 11_644_473_600 * 10_000_000;

/// The 2.5 channel, matching the pinned SDK metadata the bindings were generated from.
#[cfg(target_os = "windows")]
const RUNTIME_CHANNEL: u32 = 0x0002_0005;

/// `MddBootstrapInitializeOptions_OnPackageIdentity_NOOP` (`0x0010` in `MddBootstrap.h`), so a
/// packaged build skips the bootstrap and a failure comes back as an `HRESULT`.
#[cfg(target_os = "windows")]
const ON_PACKAGE_IDENTITY_NOOP: u32 = 0x0010;

/// `E_FAIL`, the only code a delivery error has to report back to the runtime.
#[cfg(target_os = "windows")]
const E_FAIL: windows_core::HRESULT =
    windows_core::HRESULT(i32::from_le_bytes([0x05, 0x40, 0x00, 0x80]));

/// The bound `show` waits for the App SDK's `Show` to answer before reporting `Error::Platform`.
#[cfg(target_os = "windows")]
const SHOW_BOUND: Duration = Duration::from_secs(5);

/// The channel expiry from its `Windows.Foundation.DateTime` ticks, 100-nanosecond intervals
/// from 1601, or `None` when the value is too far from the epoch for a `SystemTime`.
fn channel_expiry(universal_time: i64) -> Option<SystemTime> {
    let nanos = (i128::from(universal_time) - i128::from(EPOCH_1601_TO_1970)) * 100;
    if nanos >= 0 {
        u64::try_from(nanos)
            .ok()
            .map(|nanos| SystemTime::UNIX_EPOCH + Duration::from_nanos(nanos))
    } else {
        u64::try_from(-nanos)
            .ok()
            .map(|nanos| SystemTime::UNIX_EPOCH - Duration::from_nanos(nanos))
    }
}

/// The key of the one launch argument that carries a notification's `data`.
const DATA_KEY: &str = "pushups-data";

/// The value the builder's `AddArgument` takes for a notification's `data`, base64url so the
/// builder's own encoding is a no-op.
fn launch_argument(data: &str) -> String {
    crate::token::base64url(data.as_bytes())
}

/// The `data` bytes a launch attribute carries, or none when it lacks the key or its value is
/// not base64url.
fn launch_argument_data(argument: &str) -> Option<Vec<u8>> {
    for (key, value) in argument.split(';').filter_map(|pair| pair.split_once('=')) {
        if key == DATA_KEY {
            return crate::token::base64url_decode(value);
        }
    }
    None
}

/// What `install` sets up, once per process.
#[cfg(target_os = "windows")]
struct Runtime {
    remote_id: windows_core::GUID,
    notifications: Option<AppNotifications>,
    delivered: Delivered,
}

#[cfg(target_os = "windows")]
static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// One thread runs the bootstrapper and the registration, so the others see the result.
#[cfg(target_os = "windows")]
static SETUP: Mutex<()> = Mutex::new(());

/// The bootstrapper's entry point, from the DLL next to the app's exe. Loaded at run time, so an
/// app shipped without it still starts and gets an error from `install`.
#[cfg(target_os = "windows")]
fn bootstrapper() -> Result<ffi::BootstrapInitialize2, Error> {
    const DLL: &str = "Microsoft.WindowsAppRuntime.Bootstrap.dll";
    let name: Vec<u16> = DLL.encode_utf16().chain([0]).collect();
    // SAFETY: `name` is NUL-terminated UTF-16 that outlives the call.
    let module = unsafe { ffi::LoadLibraryW(name.as_ptr()) };
    if module.is_null() {
        return Err(Error::Platform(format!(
            "`{DLL}` is not next to the app's exe"
        )));
    }
    // SAFETY: `module` is a loaded library, never freed, and the name is a NUL-terminated C string.
    let symbol = unsafe { ffi::GetProcAddress(module, c"MddBootstrapInitialize2".as_ptr().cast()) }
        .ok_or_else(|| Error::Platform(format!("`{DLL}` has no `MddBootstrapInitialize2`")))?;
    // SAFETY: the export has the signature `MddBootstrap.h` declares, which `BootstrapInitialize2` spells.
    Ok(unsafe {
        core::mem::transmute::<unsafe extern "system" fn() -> isize, ffi::BootstrapInitialize2>(
            symbol,
        )
    })
}

/// Delivers one push, from `PushReceived` or from a push activation.
#[cfg(target_os = "windows")]
fn deliver(
    push: &bindings::PushNotificationReceivedEventArgs,
    started_app: bool,
) -> Result<(), Error> {
    let deferral = push.GetDeferral().ok();
    let payload = push.Payload().map_err(Error::platform).map(|payload| {
        let payload: Vec<u8> = payload.to_vec();
        let first = delivered().first_delivery(&payload);
        // A delivered set that cannot be read or written lets the push through, since a repeat beats a loss.
        if !first.unwrap_or(true) {
            return;
        }
        let message = Message {
            payload,
            started_app,
        };
        DISPATCHER.emit(Event::Message(message.clone()));
        crate::__private::run_background_handler(message);
    });
    // The deferral keeps the task alive while the handler runs, so it completes last.
    if let Some(deferral) = deferral.as_ref() {
        let _ = deferral.Complete();
    }
    payload
}

/// The `PushReceived` handler, from the `WinRT` thread the push arrives on.
#[cfg(target_os = "windows")]
// The callback shape is fixed by `TypedEventHandler::new`, which hands the `Ref`s over by value.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the handler signature is the one the runtime invokes"
)]
fn on_push_received(
    _sender: windows_core::Ref<bindings::PushNotificationManager>,
    args: windows_core::Ref<bindings::PushNotificationReceivedEventArgs>,
) -> windows_core::Result<()> {
    // The process was already running, so the app did not start for this push.
    let Some(args) = args.as_ref() else {
        return Ok(());
    };
    deliver(args, false).map_err(|error| windows_core::Error::new(E_FAIL, error.to_string()))
}

/// The delivered set, published by `install` before any delivery.
#[cfg(target_os = "windows")]
fn delivered() -> &'static Delivered {
    &RUNTIME
        .get()
        .expect("install publishes the runtime before any delivery")
        .delivered
}

/// The delivered set's file, in the per-user data of the app's remote id.
#[cfg(target_os = "windows")]
fn delivered_path(remote_id: &str) -> Result<PathBuf, Error> {
    let local = env::var_os("LOCALAPPDATA")
        .ok_or_else(|| Error::Platform("`LOCALAPPDATA` is not set".to_owned()))?;
    Ok(PathBuf::from(local)
        .join(remote_id)
        .join("pushups")
        .join("delivered"))
}

/// Delivers one tap on a notification, from `NotificationInvoked` or from a tap activation.
///
/// A tap on a notification without `data` emits no event, the Apple local-notification
/// precedent.
#[cfg(target_os = "windows")]
fn deliver_tap(
    args: &bindings::AppNotificationActivatedEventArgs,
    started_app: bool,
) -> Result<(), Error> {
    let argument = args.Argument().map_err(Error::platform)?;
    let Some(payload) = launch_argument_data(&argument.to_string()) else {
        return Ok(());
    };
    // A delivered set that cannot be read or written lets the tap through, since a repeat beats a loss.
    if !delivered().first_delivery(&payload).unwrap_or(true) {
        return Ok(());
    }
    DISPATCHER.emit(Event::Message(Message {
        payload,
        started_app,
    }));
    Ok(())
}

/// The `NotificationInvoked` handler, from the thread the tap arrives on.
#[cfg(target_os = "windows")]
// The callback shape is fixed by `TypedEventHandler::new`, which hands the `Ref`s over by value.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the handler signature is the one the runtime invokes"
)]
fn on_notification_invoked(
    _sender: windows_core::Ref<bindings::AppNotificationManager>,
    args: windows_core::Ref<bindings::AppNotificationActivatedEventArgs>,
) -> windows_core::Result<()> {
    // The process was already running, so the app did not start for this tap.
    let Some(args) = args.as_ref() else {
        return Ok(());
    };
    deliver_tap(args, false).map_err(|error| windows_core::Error::new(E_FAIL, error.to_string()))
}

/// The `file:///` URI the App SDK takes for an icon at a local path, or the icon as given when it
/// is a URI already.
fn icon_uri_text(icon: &str) -> String {
    if icon.contains("://") {
        icon.to_owned()
    } else {
        format!(
            "file:///{}",
            icon.replace('\\', "/").trim_start_matches('/')
        )
    }
}

/// The `Uri` of the app's icon.
#[cfg(target_os = "windows")]
fn icon_uri(icon: &str) -> Result<windows::Foundation::Uri, Error> {
    windows::Foundation::Uri::CreateUri(&icon_uri_text(icon).into()).map_err(Error::platform)
}

/// Shows the notification in the notification centre, its `Show` call on a worker thread.
#[cfg(target_os = "windows")]
pub(crate) fn show(notification: &Notification) -> Result<(), Error> {
    let Some(runtime) = RUNTIME.get() else {
        return Err(Error::NotConfigured);
    };
    // Without the display name and icon, `install` never registered the manager.
    if runtime.notifications.is_none() {
        return Err(Error::NotConfigured);
    }
    // The worker thread outlives this borrow, so it keeps its own values.
    let title = notification.title.clone();
    let body = notification.body.clone();
    let icon = notification.icon.clone();
    let tag = notification.tag.clone();
    let data = notification.data.clone();
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let result = show_blocking(
            &title,
            body.as_deref(),
            icon.as_deref(),
            tag.as_deref(),
            data.as_deref(),
        );
        let _ = tx.send(result);
    });
    let result = rx.recv_timeout(SHOW_BOUND).map_err(|_| {
        Error::Platform("the App SDK's `Show` did not answer within 5 s".to_owned())
    })?;
    // Past the bound the worker may still run, and it ends without a joiner.
    drop(handle);
    result
}

/// Builds the notification and calls `Show`, everything the worker thread does.
#[cfg(target_os = "windows")]
fn show_blocking(
    title: &str,
    body: Option<&str>,
    icon: Option<&str>,
    tag: Option<&str>,
    data: Option<&str>,
) -> Result<(), Error> {
    let manager = bindings::AppNotificationManager::Default().map_err(Error::platform)?;
    let mut builder = bindings::AppNotificationBuilder::new().map_err(Error::platform)?;
    builder = builder.AddText(&title.into()).map_err(Error::platform)?;
    if let Some(body) = body {
        builder = builder.AddText(&body.into()).map_err(Error::platform)?;
    }
    if let Some(icon) = icon {
        builder = builder
            .SetAppLogoOverride(&icon_uri(icon)?)
            .map_err(Error::platform)?;
    }
    if let Some(tag) = tag {
        builder = builder.SetTag(&tag.into()).map_err(Error::platform)?;
    }
    if let Some(data) = data {
        builder = builder
            .AddArgument(&DATA_KEY.into(), &launch_argument(data).into())
            .map_err(Error::platform)?;
    }
    let notification = builder.BuildNotification().map_err(Error::platform)?;
    manager.Show(&notification).map_err(Error::platform)
}

#[cfg(target_os = "windows")]
pub(crate) struct Windows;

#[cfg(target_os = "windows")]
impl Backend for Windows {
    fn install(config: Config) -> Result<(), Error> {
        let wns = config.windows.ok_or(Error::NotConfigured)?;
        let guard = SETUP.lock();
        if RUNTIME.get().is_some() {
            return Ok(());
        }
        let remote_id = windows_core::GUID::try_from(wns.remote_id.as_str()).map_err(|_| {
            Error::Platform(format!(
                "the Windows remote_id `{}` is not a GUID",
                wns.remote_id
            ))
        })?;
        // No COM apartment here, as tao's drag and drop needs this thread single-threaded and `windows-core` joins the implicit multithreaded one itself.
        let initialize = bootstrapper()?;
        // SAFETY: The 2.5 channel, null tag, zeroed minimum version and NOOP flag are the documented caller values.
        let status = unsafe {
            initialize(
                RUNTIME_CHANNEL,
                core::ptr::null(),
                ffi::PackageVersion {
                    major: 0,
                    minor: 0,
                    build: 0,
                    revision: 0,
                },
                ON_PACKAGE_IDENTITY_NOOP,
            )
        };
        if status.is_err() {
            return Err(Error::Platform(format!(
                "the Windows App SDK runtime could not be started: {status}"
            )));
        }
        let manager = bindings::PushNotificationManager::Default().map_err(Error::platform)?;
        if !bindings::PushNotificationManager::IsSupported().map_err(Error::platform)? {
            return Err(Error::Unsupported);
        }
        let handler = TypedEventHandler::<
            bindings::PushNotificationManager,
            bindings::PushNotificationReceivedEventArgs,
        >::new(on_push_received);
        // The handler must be in before `Register()`, or the runtime throws a COM exception.
        let _ = manager.PushReceived(&handler).map_err(Error::platform)?;
        manager.Register().map_err(Error::platform)?;
        if let Some(notifications) = &wns.notifications {
            let notification_manager =
                bindings::AppNotificationManager::Default().map_err(Error::platform)?;
            let tap_handler = TypedEventHandler::<
                bindings::AppNotificationManager,
                bindings::AppNotificationActivatedEventArgs,
            >::new(on_notification_invoked);
            // The handler must be in before the manager's `Register`, or the runtime starts a fresh process for every interaction.
            let _ = notification_manager
                .NotificationInvoked(&tap_handler)
                .map_err(Error::platform)?;
            notification_manager
                .Register2(
                    &notifications.display_name.as_str().into(),
                    &icon_uri(&notifications.icon)?,
                )
                .map_err(Error::platform)?;
        }
        let delivered_path = delivered_path(&wns.remote_id)?;
        if let Some(parent) = delivered_path.parent() {
            DirBuilder::new()
                .recursive(true)
                .create(parent)
                .map_err(Error::platform)?;
        }
        // Published before the push that started the app is handled, so a handler that calls
        // `register` finds it, and the lock released, so one that calls `install` returns at once.
        let _ = RUNTIME.set(Runtime {
            remote_id,
            notifications: wns.notifications.clone(),
            delivered: Delivered::new(delivered_path),
        });
        drop(guard);
        let args = bindings::AppInstance::GetCurrent()
            .map_err(Error::platform)?
            .GetActivatedEventArgs()
            .map_err(Error::platform)?;
        match args.Kind().map_err(Error::platform)? {
            bindings::ExtendedActivationKind::Push => {
                let push = args
                    .Data()
                    .map_err(Error::platform)?
                    .cast::<bindings::PushNotificationReceivedEventArgs>()
                    .map_err(Error::platform)?;
                deliver(&push, true)?;
            }
            // The tap that started the process carries its notification's `data`.
            bindings::ExtendedActivationKind::AppNotification => {
                let tap = args
                    .Data()
                    .map_err(Error::platform)?
                    .cast::<bindings::AppNotificationActivatedEventArgs>()
                    .map_err(Error::platform)?;
                deliver_tap(&tap, true)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn register() -> Result<(), Error> {
        let Some(runtime) = RUNTIME.get() else {
            return Err(Error::NotConfigured);
        };
        // WNS can take up to fifteen minutes to answer, so the request runs off this thread.
        thread::spawn(move || {
            DISPATCHER.emit(match create_channel(runtime.remote_id) {
                Ok(token) => Event::Token(token),
                Err(error) => Event::RegistrationFailed(error),
            });
        });
        Ok(())
    }

    /// WNS raw pushes need no user permission, so the answer is whether push can register at all.
    fn request_permission() -> impl Future<Output = Result<Permission, Error>> {
        let granted =
            RUNTIME.get().is_some() && bindings::PushNotificationManager::IsSupported() == Ok(true);
        std::future::ready(Ok(if granted {
            Permission::Granted
        } else {
            Permission::Denied
        }))
    }
}

/// Creates the fresh channel, waiting for WNS's answer.
#[cfg(target_os = "windows")]
fn create_channel(remote_id: windows_core::GUID) -> Result<Token, Error> {
    let create = bindings::PushNotificationManager::Default()
        .and_then(|manager| manager.CreateChannelAsync(remote_id))
        .and_then(|operation| operation.join())
        .map_err(Error::platform)?;
    let status = create.Status().map_err(Error::platform)?;
    if status != bindings::PushNotificationChannelStatus::CompletedSuccess {
        let extended = create.ExtendedError().unwrap_or(windows_core::HRESULT(0));
        return Err(Error::Platform(format!(
            "WNS failed the channel: {status:?} {extended}"
        )));
    }
    let channel = create.Channel().map_err(Error::platform)?;
    let channel_uri = channel
        .Uri()
        .and_then(|uri| uri.ToString())
        .map_err(Error::platform)?
        .to_string();
    let ticks = channel
        .ExpirationTime()
        .map_err(Error::platform)?
        .UniversalTime;
    let expires = channel_expiry(ticks)
        .ok_or_else(|| Error::Platform(format!("the channel expiry is out of range: {ticks}")))?;
    Ok(Token::Wns {
        channel_uri,
        expires,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_icon_path_becomes_a_file_uri_with_three_slashes() {
        assert_eq!(
            icon_uri_text(r"C:\apps\pushups\icon.png"),
            "file:///C:/apps/pushups/icon.png"
        );
        assert_eq!(
            icon_uri_text("ms-appx:///Assets/icon.png"),
            "ms-appx:///Assets/icon.png"
        );
    }

    #[test]
    fn the_epoch_of_1601_ticks_is_the_unix_epoch() {
        assert_eq!(
            channel_expiry(EPOCH_1601_TO_1970),
            Some(SystemTime::UNIX_EPOCH)
        );
    }

    #[test]
    fn an_expiry_after_the_epoch_adds_to_it() {
        // 1970-01-02 00:00:00 UTC.
        let expiry = channel_expiry(EPOCH_1601_TO_1970 + 86_400 * 10_000_000);
        assert_eq!(
            expiry,
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(86_400))
        );
    }

    #[test]
    fn an_expiry_before_the_epoch_subtracts_from_it() {
        // 1969-12-31 00:00:00 UTC, one day before the Unix epoch.
        let expiry = channel_expiry(EPOCH_1601_TO_1970 - 86_400 * 10_000_000);
        assert_eq!(
            expiry,
            Some(SystemTime::UNIX_EPOCH - Duration::from_secs(86_400))
        );
    }

    #[test]
    fn launch_argument_round_trips_through_its_key() {
        let mut state = 1u32;
        for length in [0usize, 1, 5, 63, 1000, 4096] {
            let bytes: Vec<u8> = (0..length)
                .map(|_| {
                    state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    state.to_be_bytes()[0]
                })
                .collect();
            let data = String::from_utf8_lossy(&bytes).to_string();
            let argument = format!("{DATA_KEY}={}", launch_argument(&data));
            assert_eq!(
                launch_argument_data(&argument),
                Some(data.as_bytes().to_vec())
            );
        }
    }

    #[test]
    fn launch_argument_uses_no_character_the_shell_has_to_escape() {
        let data = "a;= &b\tc";
        assert!(
            launch_argument(data)
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    #[test]
    fn a_launch_attribute_without_our_key_carries_no_data() {
        assert_eq!(launch_argument_data("other=1"), None);
        assert_eq!(launch_argument_data(""), None);
    }

    #[test]
    fn launch_argument_data_reads_our_key_from_among_others() {
        let value = launch_argument("payload");
        let argument = format!("other=1;{DATA_KEY}={value};third=2");
        assert_eq!(launch_argument_data(&argument), Some(b"payload".to_vec()));
    }

    #[test]
    fn launch_argument_data_rejects_a_value_outside_the_alphabet() {
        assert_eq!(launch_argument_data(&format!("{DATA_KEY}=ab+cd")), None);
    }

    #[test]
    fn a_ticks_value_farther_than_a_duration_cannot_hold_is_rejected() {
        assert_eq!(channel_expiry(i64::MAX), None);
        assert_eq!(channel_expiry(i64::MIN), None);
    }
}
