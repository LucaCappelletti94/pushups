//! `install` and `register` on a Windows machine without the Windows App SDK runtime beside the
//! test binary, as an app shipped without `Microsoft.WindowsAppRuntime.Bootstrap.dll` finds them.
#![cfg(target_os = "windows")]

use pushups::{Config, Error, WnsConfig};

const REMOTE_ID: &str = "6f9619ff-8b86-d011-b42d-00c04fc964ff";

#[test]
fn install_without_a_windows_part_is_not_configured() {
    assert_eq!(pushups::install(Config::new()), Err(Error::NotConfigured));
}

#[test]
fn install_names_a_remote_id_that_is_not_a_guid() {
    let error = pushups::install(Config::new().windows(WnsConfig::new("not-a-guid"))).unwrap_err();
    assert!(
        matches!(&error, Error::Platform(message) if message.contains("not-a-guid")),
        "{error:?}"
    );
}

#[test]
fn install_without_the_bootstrapper_names_it_and_the_app_still_runs() {
    let error = pushups::install(Config::new().windows(WnsConfig::new(REMOTE_ID))).unwrap_err();
    assert!(
        matches!(&error, Error::Platform(message) if message.contains("Microsoft.WindowsAppRuntime.Bootstrap.dll")),
        "{error:?}"
    );
    // Nothing was set up, so a registration is refused rather than sent.
    assert_eq!(pushups::register(), Err(Error::NotConfigured));
}
