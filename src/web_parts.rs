//! The parts of the web backend that need no browser.

use std::time::{Duration, UNIX_EPOCH};

use crate::{Error, Permission, Token};

/// The script URL the page registers, telling the shim its mode in the query.
fn with_mode(script: &str, mode: &str) -> String {
    let separator = if script.contains('?') { '&' } else { '?' };
    format!("{script}{separator}pushups={mode}")
}

/// The URL a page registers when a Rust handler runs in the worker, `script` being the app's own
/// entry module or the wasm-bindgen glue.
pub(crate) fn rust_worker_url(script: &str) -> String {
    with_mode(script, "rust")
}

/// The URL a page registers for the static worker: the app's own path, else the one `dx` bundled
/// from this crate, else [`WebPushConfig::DEFAULT_SERVICE_WORKER_PATH`](crate::WebPushConfig::DEFAULT_SERVICE_WORKER_PATH).
/// `bundled` runs only when the app set no path.
pub(crate) fn static_worker_url(
    configured: Option<&str>,
    bundled: impl FnOnce() -> Option<String>,
) -> String {
    match configured {
        Some(path) => with_mode(path, "static"),
        None => match bundled() {
            Some(path) => with_mode(&path, "static"),
            None => with_mode(crate::WebPushConfig::DEFAULT_SERVICE_WORKER_PATH, "static"),
        },
    }
}

/// A subscription key of exactly `N` bytes, which `fill` copies from the browser's array of
/// `length` bytes once the length is right.
pub(crate) fn subscription_key<const N: usize>(
    name: &str,
    length: u32,
    fill: impl FnOnce(&mut [u8; N]),
) -> Result<[u8; N], Error> {
    if usize::try_from(length).ok() != Some(N) {
        return Err(Error::Platform(format!(
            "{name} of {length} bytes, not {N}"
        )));
    }
    let mut key = [0; N];
    fill(&mut key);
    Ok(key)
}

/// The token of a push subscription, from its endpoint, keys and expiry in milliseconds.
pub(crate) fn web_push_token(
    endpoint: String,
    p256dh: [u8; 65],
    auth: [u8; 16],
    expiration_ms: Option<f64>,
) -> Result<Token, Error> {
    let expires = expiration_ms
        .map(|ms| {
            Duration::try_from_secs_f64(ms / 1000.0)
                .map(|since| UNIX_EPOCH + since)
                .map_err(|_| Error::Platform(format!("subscription expiry {ms} is not a time")))
        })
        .transpose()?;
    Ok(Token::WebPush {
        endpoint,
        p256dh,
        auth,
        expires,
    })
}

/// The answer of `Notification.requestPermission()`.
pub(crate) fn permission_from(answer: &str) -> Result<Permission, Error> {
    match answer {
        "granted" => Ok(Permission::Granted),
        "denied" => Ok(Permission::Denied),
        "default" => Ok(Permission::Dismissed),
        other => Err(Error::Platform(format!(
            "unknown notification permission {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn the_worker_url_names_its_mode_and_keeps_an_existing_query() {
        assert_eq!(
            rust_worker_url("https://a.example/assets/app-dxh1.js"),
            "https://a.example/assets/app-dxh1.js?pushups=rust"
        );
        assert_eq!(rust_worker_url("/sw.js?v=3"), "/sw.js?v=3&pushups=rust");
        assert_eq!(
            static_worker_url(Some("/sw.js?v=3"), || None),
            "/sw.js?v=3&pushups=static"
        );
    }

    #[test]
    fn the_static_worker_is_the_apps_path_then_the_bundled_one_then_the_default() {
        let bundled = || Some("/assets/pushups-sw.js".to_owned());
        assert_eq!(
            static_worker_url(Some("/sw.js"), || unreachable!("a configured path wins")),
            "/sw.js?pushups=static"
        );
        assert_eq!(
            static_worker_url(None, bundled),
            "/assets/pushups-sw.js?pushups=static"
        );
        assert_eq!(
            static_worker_url(None, || None),
            "/pushups-sw.js?pushups=static"
        );
        assert_eq!(
            static_worker_url(Some("/pushups-sw.js"), bundled),
            "/pushups-sw.js?pushups=static"
        );
    }

    #[test]
    fn a_subscription_becomes_a_web_push_token() {
        let token = web_push_token(
            "https://push.example/sub".to_owned(),
            [4; 65],
            [7; 16],
            Some(1_500.0),
        )
        .unwrap();
        assert_eq!(
            token,
            Token::WebPush {
                endpoint: "https://push.example/sub".to_owned(),
                p256dh: [4; 65],
                auth: [7; 16],
                expires: Some(UNIX_EPOCH + Duration::from_millis(1_500)),
            }
        );
    }

    #[test]
    fn a_subscription_without_expiry_has_none() {
        let token = web_push_token("e".to_owned(), [4; 65], [7; 16], None).unwrap();
        assert!(matches!(token, Token::WebPush { expires: None, .. }));
    }

    #[test]
    fn keys_of_the_wrong_length_are_refused_before_any_copy() {
        let refused = subscription_key::<65>("p256dh key", 64, |_| {
            unreachable!("a key of the wrong length is never copied")
        });
        assert_eq!(
            refused,
            Err(Error::Platform("p256dh key of 64 bytes, not 65".to_owned()))
        );
        assert_eq!(
            subscription_key::<16>("auth secret", 16, |key| key.fill(7)),
            Ok([7; 16])
        );
    }

    #[test]
    fn an_expiry_that_is_not_a_time_is_refused() {
        for bad in [f64::NAN, f64::INFINITY, -1.0] {
            assert!(web_push_token("e".to_owned(), [4; 65], [7; 16], Some(bad)).is_err());
        }
    }

    #[test]
    fn browser_permission_answers_map_to_permission() {
        assert_eq!(permission_from("granted"), Ok(Permission::Granted));
        assert_eq!(permission_from("denied"), Ok(Permission::Denied));
        assert_eq!(permission_from("default"), Ok(Permission::Dismissed));
        assert!(permission_from("prompt").is_err());
    }
}
