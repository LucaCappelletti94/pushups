#![doc = include_str!("../README.md")]

use tauri::Runtime;
use tauri::plugin::{Builder, TauriPlugin};

/// The crate the app calls, at the version whose Android module the plugin ships.
pub use pushups;

/// The plugin, which a Tauri app adds with `.plugin(tauri_plugin_pushups::init())` so its build
/// includes the crate's Android module and its iOS push entitlement. The app itself calls
/// `pushups`.
#[must_use]
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("pushups").build()
}
