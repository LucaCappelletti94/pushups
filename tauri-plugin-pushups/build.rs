//! Copies `pushups`' Android module into `OUT_DIR` and hands the copy to Tauri, so Tauri's own
//! files land there and never in cargo's registry. Names the app's library for the module, and
//! adds the iOS push entitlement and the `remote-notification` background mode.

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
    // iOS apps build only on macOS, the one host where Tauri can edit their entitlements and Info.plist.
    #[cfg(target_os = "macos")]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") {
        // An app's own value stays. Without one, Xcode replaces `development` with the value of
        // the provisioning profile it signs with, so an export for distribution reads `production`.
        tauri_plugin::mobile::update_entitlements(|entitlements| {
            if !entitlements.contains_key("aps-environment") {
                entitlements.insert("aps-environment".into(), "development".into());
            }
        })
        .expect("the app's entitlements are updated");
        // A push delivered with the app out of the foreground needs the mode, and the project may
        // already list its own, so the list keeps what it has and gains the mode once.
        tauri_plugin::mobile::update_info_plist(|info| {
            let remote = plist::Value::String("remote-notification".to_owned());
            match info.get_mut("UIBackgroundModes") {
                Some(plist::Value::Array(modes)) => {
                    if !modes.contains(&remote) {
                        modes.push(remote);
                    }
                }
                _ => {
                    info.insert(
                        "UIBackgroundModes".into(),
                        plist::Value::Array(vec![remote]),
                    );
                }
            }
        })
        .expect("the app's Info.plist is updated");
    }
    println!("cargo::rerun-if-env-changed=DEP_PUSHUPS_ANDROID");
    println!("cargo::rerun-if-env-changed=WRY_ANDROID_LIBRARY");
    rerun_with_tauri_projects();
    tauri_plugin::Builder::new(&[]).android_path(copy).build();
}

/// Asks cargo to run this script again when the Tauri CLI names another generated project, or
/// when `tauri android init` or `tauri ios init` rewrites a file this script edits, since the
/// edit lives in that file and a cached run would leave it out.
fn rerun_with_tauri_projects() {
    println!("cargo::rerun-if-env-changed=TAURI_ANDROID_PROJECT_PATH");
    println!("cargo::rerun-if-env-changed=TAURI_IOS_PROJECT_PATH");
    println!("cargo::rerun-if-env-changed=TAURI_IOS_APP_NAME");
    let mut edited = Vec::new();
    if let Some(project) = std::env::var_os("TAURI_ANDROID_PROJECT_PATH") {
        edited.push(Path::new(&project).join("app/src/main/AndroidManifest.xml"));
    }
    if let (Some(project), Ok(app)) = (
        std::env::var_os("TAURI_IOS_PROJECT_PATH"),
        std::env::var("TAURI_IOS_APP_NAME"),
    ) {
        let ios = Path::new(&project).join(format!("{app}_iOS"));
        edited.push(ios.join(format!("{app}_iOS.entitlements")));
        edited.push(ios.join("Info.plist"));
    }
    // A missing path would make cargo rerun this script on every build.
    for file in edited.iter().filter(|file| file.exists()) {
        println!("cargo::rerun-if-changed={}", file.display());
    }
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
