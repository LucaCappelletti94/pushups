//! The parts of the web backend that need no browser.

use std::time::{Duration, UNIX_EPOCH};

use crate::{Error, Permission, Token};

/// Whether a Rust handler will run in the service worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerMode {
    /// The static shim alone.
    Static,
    /// The app's glue, whose `main` serves a Rust handler.
    Rust,
}

/// The script URL the page registers, telling the shim its mode in the query.
pub(crate) fn worker_script_url(script: &str, mode: WorkerMode) -> String {
    let separator = if script.contains('?') { '&' } else { '?' };
    let mode = match mode {
        WorkerMode::Static => "static",
        WorkerMode::Rust => "rust",
    };
    format!("{script}{separator}pushups={mode}")
}

/// The token of a push subscription, from its endpoint, keys and expiry in milliseconds.
pub(crate) fn web_push_token(
    endpoint: String,
    p256dh: &[u8],
    auth: &[u8],
    expiration_ms: Option<f64>,
) -> Result<Token, Error> {
    let p256dh = p256dh
        .try_into()
        .map_err(|_| Error::Platform(format!("p256dh key of {} bytes, not 65", p256dh.len())))?;
    let auth = auth
        .try_into()
        .map_err(|_| Error::Platform(format!("auth secret of {} bytes, not 16", auth.len())))?;
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
            worker_script_url("/pushups-sw.js", WorkerMode::Static),
            "/pushups-sw.js?pushups=static"
        );
        assert_eq!(
            worker_script_url("https://a.example/assets/app-dxh1.js", WorkerMode::Rust),
            "https://a.example/assets/app-dxh1.js?pushups=rust"
        );
        assert_eq!(
            worker_script_url("/sw.js?v=3", WorkerMode::Static),
            "/sw.js?v=3&pushups=static"
        );
    }

    #[test]
    fn a_subscription_becomes_a_web_push_token() {
        let token = web_push_token(
            "https://push.example/sub".to_owned(),
            &[4; 65],
            &[7; 16],
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
        let token = web_push_token("e".to_owned(), &[4; 65], &[7; 16], None).unwrap();
        assert!(matches!(token, Token::WebPush { expires: None, .. }));
    }

    #[test]
    fn keys_of_the_wrong_length_are_refused() {
        assert!(web_push_token("e".to_owned(), &[4; 64], &[7; 16], None).is_err());
        assert!(web_push_token("e".to_owned(), &[4; 65], &[7; 15], None).is_err());
    }

    #[test]
    fn an_expiry_that_is_not_a_time_is_refused() {
        for bad in [f64::NAN, f64::INFINITY, -1.0] {
            assert!(web_push_token("e".to_owned(), &[4; 65], &[7; 16], Some(bad)).is_err());
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
