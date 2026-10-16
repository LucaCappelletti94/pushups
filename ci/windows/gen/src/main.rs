//! Regenerates `src/windows/bindings.rs` from the four Windows App SDK `.winmd` files
//! named on the command line.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [push, lifecycle, notifications, notification_builder] = args.as_slice() else {
        eprintln!(
            "usage: windows-bindings-gen <Microsoft.Windows.PushNotifications.winmd> <Microsoft.Windows.AppLifecycle.winmd> <Microsoft.Windows.AppNotifications.winmd> <Microsoft.Windows.AppNotifications.Builder.winmd>"
        );
        std::process::exit(1);
    };
    let out = "src/windows/bindings.rs";
    let argv: Vec<&str> = vec![
        "--in",
        push,
        "--in",
        lifecycle,
        // The App Notifications API the `show` and tap paths reach.
        "--in",
        notifications,
        "--in",
        notification_builder,
        // The OS metadata the WinAppSDK types reference.
        "--in",
        "default",
        // OS types outside the four namespaces resolve through the `windows` crate.
        "--reference",
        "windows",
        "--out",
        out,
        "--flat",
        // The classes and enums `src/windows.rs` reaches. The generator skips a method whose types
        // are not listed, and its warnings fail this run, so a missing one is a visible error.
        "--filter",
        "Microsoft.Windows.PushNotifications.PushNotificationManager",
        "--filter",
        "Microsoft.Windows.PushNotifications.PushNotificationCreateChannelResult",
        "--filter",
        "Microsoft.Windows.PushNotifications.PushNotificationCreateChannelStatus",
        "--filter",
        "Microsoft.Windows.PushNotifications.PushNotificationChannel",
        "--filter",
        "Microsoft.Windows.PushNotifications.PushNotificationChannelStatus",
        "--filter",
        "Microsoft.Windows.PushNotifications.PushNotificationReceivedEventArgs",
        "--filter",
        "Microsoft.Windows.AppLifecycle.AppInstance",
        "--filter",
        "Microsoft.Windows.AppLifecycle.AppActivationArguments",
        "--filter",
        "Microsoft.Windows.AppLifecycle.ExtendedActivationKind",
        "--filter",
        "Microsoft.Windows.AppNotifications.AppNotificationManager",
        "--filter",
        "Microsoft.Windows.AppNotifications.AppNotification",
        "--filter",
        "Microsoft.Windows.AppNotifications.AppNotificationActivatedEventArgs",
        "Microsoft.Windows.AppNotifications.AppNotificationSetting",
        "--filter",
        "Microsoft.Windows.AppNotifications.AppNotificationPriority",
        "--filter",
        "Microsoft.Windows.AppNotifications.AppNotificationProgressData",
        "--filter",
        "Microsoft.Windows.AppNotifications.AppNotificationProgressResult",
        "--filter",
        "Microsoft.Windows.AppNotifications.Builder.AppNotificationBuilder",
        "--filter",
        "Microsoft.Windows.AppNotifications.Builder.AppNotificationButton",
        "--filter",
        "Microsoft.Windows.AppNotifications.Builder.AppNotificationButtonStyle",
        "--filter",
        "Microsoft.Windows.AppNotifications.Builder.AppNotificationAudioLooping",
        "--filter",
        "Microsoft.Windows.AppNotifications.Builder.AppNotificationComboBox",
        "--filter",
        "Microsoft.Windows.AppNotifications.Builder.AppNotificationDuration",
        "--filter",
        "Microsoft.Windows.AppNotifications.Builder.AppNotificationImageCrop",
        "--filter",
        "Microsoft.Windows.AppNotifications.Builder.AppNotificationProgressBar",
        "--filter",
        "Microsoft.Windows.AppNotifications.Builder.AppNotificationScenario",
        "--filter",
        "Microsoft.Windows.AppNotifications.Builder.AppNotificationSoundEvent",
        "--filter",
        "Microsoft.Windows.AppNotifications.Builder.AppNotificationTextProperties",
    ];
    let warnings = windows_bindgen::bindgen(&argv);
    if !warnings.is_empty() {
        eprintln!("{warnings}");
        std::process::exit(1);
    }
    relint(out);
}

// The generated `#![allow]` header predates the repository's lint rules, so it is replaced.
fn relint(out: &str) {
    // The exact lints the generated bindings trip, each named so a new one is a visible diff.
    const HEADER: &str = "#![allow(\n    non_snake_case,\n    non_upper_case_globals,\n    non_camel_case_types,\n    dead_code,\n    clippy::borrow_as_ptr,\n    clippy::missing_transmute_annotations,\n    clippy::ptr_as_ptr,\n    clippy::redundant_closure_for_method_calls,\n    clippy::transmute_ptr_to_ptr,\n    clippy::useless_transmute,\n    reason = \"generated windows-bindgen output\"\n)]";
    let source = match std::fs::read_to_string(out) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("reading {out}: {error}");
            std::process::exit(1);
        }
    };
    let Some(start) = source.find("#![allow(") else {
        eprintln!("the generated allow header is missing from {out}");
        std::process::exit(1);
    };
    let Some(close) = source[start..].find(")]") else {
        eprintln!("the generated allow header is unterminated in {out}");
        std::process::exit(1);
    };
    let fixed = format!(
        "{}{}{}",
        &source[..start],
        HEADER,
        &source[start + close + 2..]
    );
    if let Err(error) = std::fs::write(out, fixed) {
        eprintln!("writing {out}: {error}");
        std::process::exit(1);
    }
}
