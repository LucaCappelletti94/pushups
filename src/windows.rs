//! The Windows backend, WNS through the Windows App SDK's `PushNotificationManager`.
//!
//! An unpackaged app has no auto-initializer, so `install` runs the bootstrapper, checks
//! `IsSupported()`, puts the `PushReceived` handler in before `Register()`, and delivers the
//! push that started the app. `register` creates the fresh channel on a worker thread, since
//! WNS can take up to fifteen minutes to answer. Until W0 settles background activation on
//! hardware, an unpackaged app receives pushes only while it is running.

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
use std::sync::OnceLock;
#[cfg(target_os = "windows")]
use std::thread;

#[cfg(target_os = "windows")]
use parking_lot::Mutex;
#[cfg(target_os = "windows")]
use windows::Foundation::TypedEventHandler;
#[cfg(target_os = "windows")]
use windows_core::Interface as _;

#[cfg(target_os = "windows")]
use crate::dispatch::DISPATCHER;
#[cfg(target_os = "windows")]
use crate::{Config, Context, Error, Event, Message, Notification, Permission, Token};

/// The 100-nanosecond ticks from 1601 to the Unix epoch, the base of `Windows.Foundation.DateTime`.
const EPOCH_1601_TO_1970: i64 = 11_644_473_600 * 10_000_000;

/// The 2.5 channel, matching the pinned SDK metadata the bindings were generated from.
#[cfg(target_os = "windows")]
const RUNTIME_CHANNEL: u32 = 0x0002_0005;

/// `MddBootstrapInitializeOptions_OnPackageIdentity_NOOP`, so a packaged build skips it.
#[cfg(target_os = "windows")]
const BOOTSTRAP_OPTIONS: u32 = 5;

/// `E_FAIL`, the only code a delivery error has to report back to the runtime.
#[cfg(target_os = "windows")]
const E_FAIL: windows_core::HRESULT =
    windows_core::HRESULT(i32::from_le_bytes([0x05, 0x40, 0x00, 0x80]));

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

/// What `install` sets up, once per process.
#[cfg(target_os = "windows")]
struct Runtime {
    remote_id: windows_core::GUID,
}

#[cfg(target_os = "windows")]
static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// One thread runs the bootstrapper and the registration, so the others see the result.
#[cfg(target_os = "windows")]
static SETUP: Mutex<()> = Mutex::new(());

/// Maps a platform error into the crate's, keeping its description.
#[cfg(target_os = "windows")]
fn platform(error: impl std::fmt::Display) -> Error {
    Error::Platform(error.to_string())
}

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
    let payload = push.Payload().map_err(platform).map(|payload| {
        let message = Message {
            payload: payload.to_vec(),
            started_app,
        };
        DISPATCHER.emit(Event::Message(message.clone()));
        run_background_handler(message);
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

/// Runs the app's `#[background_handler]`, if it has one, catching a panic.
#[cfg(target_os = "windows")]
fn run_background_handler(message: Message) {
    if let Some(handler) = crate::__private::BACKGROUND_HANDLERS.first() {
        // The panic hook already reported a panic, and the push is out by then.
        let _ = std::panic::catch_unwind(|| handler(Context { _private: () }, message));
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn install(config: Config) -> Result<(), Error> {
    let wns = config.windows.ok_or(Error::NotConfigured)?;
    let _guard = SETUP.lock();
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
            BOOTSTRAP_OPTIONS,
        )
    };
    if status.is_err() {
        return Err(Error::Platform(format!(
            "the Windows App SDK runtime could not be started: {status}"
        )));
    }
    let manager = bindings::PushNotificationManager::Default().map_err(platform)?;
    if !bindings::PushNotificationManager::IsSupported().map_err(platform)? {
        return Err(Error::Unsupported);
    }
    let handler = TypedEventHandler::<
        bindings::PushNotificationManager,
        bindings::PushNotificationReceivedEventArgs,
    >::new(on_push_received);
    // The handler must be in before `Register()`, or the runtime throws a COM exception.
    let _ = manager.PushReceived(&handler).map_err(platform)?;
    manager.Register().map_err(platform)?;
    let args = bindings::AppInstance::GetCurrent()
        .map_err(platform)?
        .GetActivatedEventArgs()
        .map_err(platform)?;
    if args.Kind().map_err(platform)? == bindings::ExtendedActivationKind::Push {
        let push = args
            .Data()
            .map_err(platform)?
            .cast::<bindings::PushNotificationReceivedEventArgs>()
            .map_err(platform)?;
        deliver(&push, true)?;
    }
    let _ = RUNTIME.set(Runtime { remote_id });
    Ok(())
}

#[cfg(target_os = "windows")]
pub(crate) fn register() -> Result<(), Error> {
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

/// Creates the fresh channel, waiting for WNS's answer.
#[cfg(target_os = "windows")]
fn create_channel(remote_id: windows_core::GUID) -> Result<Token, Error> {
    let create = bindings::PushNotificationManager::Default()
        .and_then(|manager| manager.CreateChannelAsync(remote_id))
        .and_then(|operation| operation.join())
        .map_err(platform)?;
    let status = create.Status().map_err(platform)?;
    if status != bindings::PushNotificationChannelStatus::CompletedSuccess {
        let extended = create.ExtendedError().unwrap_or(windows_core::HRESULT(0));
        return Err(Error::Platform(format!(
            "WNS failed the channel: {status:?} {extended}"
        )));
    }
    let channel = create.Channel().map_err(platform)?;
    let channel_uri = channel
        .Uri()
        .and_then(|uri| uri.ToString())
        .map_err(platform)?
        .to_string();
    let ticks = channel.ExpirationTime().map_err(platform)?.UniversalTime;
    let expires = channel_expiry(ticks)
        .ok_or_else(|| Error::Platform(format!("the channel expiry is out of range: {ticks}")))?;
    Ok(Token::Wns {
        channel_uri,
        expires,
    })
}

/// WNS raw pushes need no user permission, so the answer is whether push can register at all.
#[cfg(target_os = "windows")]
pub(crate) fn request_permission() -> std::future::Ready<Result<Permission, Error>> {
    let granted =
        RUNTIME.get().is_some() && bindings::PushNotificationManager::IsSupported() == Ok(true);
    std::future::ready(Ok(if granted {
        Permission::Granted
    } else {
        Permission::Denied
    }))
}

/// Windows keeps the events in the dispatcher, so there is nothing to hand over here.
#[cfg(target_os = "windows")]
pub(crate) fn handler_set() {}

#[cfg(target_os = "windows")]
pub(crate) fn in_service_worker() -> bool {
    false
}

#[cfg(target_os = "windows")]
pub(crate) fn serve_service_worker<H, F>(_handler: H) -> Result<(), Error>
where
    H: Fn(Message) -> F + 'static,
    F: Future<Output = Notification> + 'static,
{
    Err(Error::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_ticks_value_farther_than_a_duration_cannot_hold_is_rejected() {
        assert_eq!(channel_expiry(i64::MAX), None);
        assert_eq!(channel_expiry(i64::MIN), None);
    }
}
