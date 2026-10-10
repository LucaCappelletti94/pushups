//! Copies `pushups`' Android module into `OUT_DIR` and hands the copy to Tauri, so Tauri's own
//! files land there and never in cargo's registry. Names the app's library for the module, and
//! adds the iOS push entitlement.

use std::fs;
use std::path::Path;

fn main() {
    let module = std::env::var_os("DEP_PUSHUPS_ANDROID")
        .expect("pushups is a direct dependency, whose build script sets DEP_PUSHUPS_ANDROID");
    let out = std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR");
    let copy = Path::new(&out).join("pushups-android");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        // Gradle builds the copy in place while Tauri's Gradle task reruns this script, so the
        // copy is updated file by file and never removed. A new `pushups` version gets a new `OUT_DIR`.
        copy_dir(Path::new(&module), &copy).expect("the Android module is copied");
        // Tauri's activity names no library, so the module learns it from the manifest.
        if let Ok(library) = std::env::var("WRY_ANDROID_LIBRARY") {
            tauri_plugin::mobile::update_android_manifest(
                "PUSHUPS LIBRARY",
                "application",
                format!(
                    r#"<meta-data android:name="pushups.lib_name" android:value="{library}" />"#
                ),
            )
            .expect("the app's manifest is updated");
        }
    }
    // iOS apps build only on macOS, the one host where Tauri can edit their entitlements.
    #[cfg(target_os = "macos")]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") {
        // A debug build signs with a development profile, a release build with a distribution one.
        let environment = if std::env::var("PROFILE").as_deref() == Ok("release") {
            "production"
        } else {
            "development"
        };
        tauri_plugin::mobile::update_entitlements(|entitlements| {
            entitlements.insert("aps-environment".into(), environment.into());
        })
        .expect("the app's entitlements are updated");
    }
    println!("cargo::rerun-if-env-changed=DEP_PUSHUPS_ANDROID");
    println!("cargo::rerun-if-env-changed=WRY_ANDROID_LIBRARY");
    tauri_plugin::Builder::new(&[]).android_path(copy).build();
}

/// Copies `from` into `to`, writing only files whose content differs, so Gradle sees no change on
/// a rerun, and leaving out the build output a local Gradle run may have left in `from`.
fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            if entry.file_name() != "build" {
                copy_dir(&entry.path(), &target)?;
            }
        } else {
            let content = fs::read(entry.path())?;
            if fs::read(&target).ok().as_deref() != Some(content.as_slice()) {
                fs::write(&target, content)?;
            }
            println!("cargo::rerun-if-changed={}", entry.path().display());
        }
    }
    Ok(())
}
