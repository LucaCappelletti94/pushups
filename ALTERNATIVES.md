# Alternatives

Rust crates that receive remote pushes in a client app, checked against their source on 2026-10-17.

| Crate | Framework | Android | iOS | macOS | Windows | Linux | Web | Version |
|---|---|---|---|---|---|---|---|---|
| [pushups](https://github.com/LucaCappelletti94/pushups) | any | FCM, UnifiedPush | APNs | APNs | WNS | UnifiedPush | Web Push | 0.1.0 |
| [tauri-plugin-notifications](https://github.com/Choochmeque/tauri-plugin-notifications) | Tauri 2 | FCM | APNs | APNs | | UnifiedPush | | 0.5.0-rc.14, 2026-09-16 |
| [tauri-plugin-push-notifications](https://github.com/spicavi/tauri-plugin-push-notifications) | Tauri 2 | FCM | APNs | | | | | 0.1.0, 2026-07-14 |
| [tauri-plugin-mobile-push](https://github.com/yanqianglu/tauri-plugin-mobile-push) | Tauri 2 | FCM | APNs | | | | | 0.1.4, 2026-04-18 |
| [tauri-plugin-remote-push](https://github.com/thing-e/tauri-plugin-remote-push) | Tauri 2 | FCM | APNs, with a hand-edited `AppDelegate` | | | | | 1.0.10, 2025-06-23 |
| [tauri-plugin-fcm](https://github.com/srod/tauri-plugin-fcm) | Tauri 2 | the FCM token, messages are only logged | the FCM token | | | | | 0.2.0, 2026-05-06 |
| [g3-native-plugins](https://github.com/g3techhq/g3-native-plugins) | Dioxus 0.7.9 exactly | FCM | APNs | | | | | 0.4.4, 2026-10-11 |
| [bevy_ios_notifications](https://github.com/rustunit/bevy_ios_notifications) | Bevy 0.19 | | the APNs token, alerts in front and taps, no background pushes | | | | | 0.8.0, 2026-08-23 |
| [unifiedpush](https://codeberg.org/UnifiedPush/unifiedpush-rs) | any | | | | | UnifiedPush | | 0.1.0, 2026-04-08 |

The Linux push of `tauri-plugin-notifications` is in its 0.5 release candidates, whose README lists Windows push as not yet available. Its last stable release, 0.4.6, has no D-Bus dependency.

winit, tao, egui, iced, Slint and Makepad have no push support of their own. On the web, Leptos and Yew reach the Push API through `web-sys` feature flags and a service worker the app writes itself.

On Windows, the `windows` crate exposes `Windows.Networking.PushNotifications`, whose channel APIs need a Store-provisioned identity. Unpackaged apps get WNS through the Windows App SDK, whose Rust bindings, [windows-app-rs](https://github.com/microsoft/windows-app-rs), were archived in 2022 without a release.

## Upstream work

Push in Tauri itself is tracked in [tauri#11651](https://github.com/tauri-apps/tauri/issues/11651), open since 2024-11 with the drafts [tauri#11652](https://github.com/tauri-apps/tauri/pull/11652) and [plugins-workspace#2066](https://github.com/tauri-apps/plugins-workspace/pull/2066). In tao, the draft [tao#1015](https://github.com/tauri-apps/tao/pull/1015) and [tao#1139](https://github.com/tauri-apps/tao/pull/1139) report the Apple push token from tao's own delegates, and [tao#1364](https://github.com/tauri-apps/tao/pull/1364), by the author of pushups, reports the pushes too. All are open.

## Neighbouring crates

[notify-rust](https://github.com/hoodie/notify-rust) on Windows, macOS, Linux and the BSDs, [tauri-plugin-notification](https://github.com/tauri-apps/plugins-workspace/tree/v2/plugins/notification) in Tauri apps, [mac-notification-sys](https://github.com/h4llow3En/mac-notification-sys) on macOS and [tauri-winrt-notification](https://github.com/tauri-apps/winrt-notification) on Windows show local notifications and receive no pushes.

[a2](https://github.com/reown-com/a2) for APNs, [fcm-service](https://github.com/real-ali/fcm-service) for FCM's HTTP v1 API, and [web-push](https://github.com/pimeys/rust-web-push) and [web-push-native](https://github.com/leotaku/web-push-native) for Web Push and UnifiedPush send pushes from a server, the other end of pushups. No crate on crates.io sends WNS, which takes one authenticated HTTPS POST to the channel URI.
