//! The parts of the web backend that need no browser.

use std::borrow::Cow;
use std::time::{Duration, UNIX_EPOCH};

use crate::{Error, Permission, Token};

/// The static worker's script: the app's own path, else the one `dx` bundled from this crate,
/// else [`WebPushConfig::DEFAULT_SERVICE_WORKER_PATH`](crate::WebPushConfig::DEFAULT_SERVICE_WORKER_PATH).
/// `bundled` runs only when the app set no path.
pub(crate) fn static_worker_script(
    configured: Option<&str>,
    bundled: impl FnOnce() -> Option<String>,
) -> Cow<'_, str> {
    match configured {
        Some(path) => Cow::Borrowed(path),
        None => bundled().map_or(
            Cow::Borrowed(crate::WebPushConfig::DEFAULT_SERVICE_WORKER_PATH),
            Cow::Owned,
        ),
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

/// The state of `Notification.permission`, which the browser answers from its own record.
pub(crate) fn permission_state_from(state: &str) -> Result<Permission, Error> {
    match state {
        "granted" => Ok(Permission::Granted),
        "denied" => Ok(Permission::Denied),
        "default" => Ok(Permission::NotDetermined),
        other => Err(Error::Platform(format!(
            "unknown notification permission state {other:?}"
        ))),
    }
}

/// How the page's subscribes stand, which decides whether a permission grant subscribes again.
#[derive(Debug, Default)]
pub(crate) struct Subscribing {
    in_flight: usize,
    last: Option<bool>,
    retry: bool,
}

impl Subscribing {
    /// A subscribe starts, for a `register` or a retry.
    pub(crate) fn started(&mut self) {
        self.in_flight += 1;
    }

    /// A subscribe settled, `subscribed` telling whether it gave a subscription. True when the
    /// page should subscribe again, for a grant that came while this one was running.
    pub(crate) fn settled(&mut self, subscribed: bool) -> bool {
        self.in_flight = self.in_flight.saturating_sub(1);
        self.last = Some(subscribed);
        if subscribed {
            self.retry = false;
            return false;
        }
        let again = self.retry && self.in_flight == 0;
        if again {
            self.retry = false;
        }
        again
    }

    /// The user granted permission. True when the page should subscribe now, after a failed
    /// subscribe the browser refused for want of that permission.
    pub(crate) fn granted(&mut self) -> bool {
        if self.in_flight > 0 {
            self.retry = true;
            return false;
        }
        self.last == Some(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn the_static_worker_is_the_apps_path_then_the_bundled_one_then_the_default() {
        let bundled = || Some("/assets/pushups-sw.js".to_owned());
        assert_eq!(
            static_worker_script(Some("/sw.js"), || unreachable!("a configured path wins")),
            "/sw.js"
        );
        assert_eq!(static_worker_script(None, bundled), "/assets/pushups-sw.js");
        assert_eq!(static_worker_script(None, || None), "/pushups-sw.js");
        assert_eq!(
            static_worker_script(Some("/pushups-sw.js"), bundled),
            "/pushups-sw.js"
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

    #[test]
    fn browser_permission_states_map_to_permission() {
        assert_eq!(permission_state_from("granted"), Ok(Permission::Granted));
        assert_eq!(permission_state_from("denied"), Ok(Permission::Denied));
        assert_eq!(
            permission_state_from("default"),
            Ok(Permission::NotDetermined)
        );
        assert!(permission_state_from("prompt").is_err());
    }

    #[test]
    fn a_grant_subscribes_again_only_after_a_failed_subscribe() {
        let mut never = Subscribing::default();
        assert!(!never.granted(), "no register asked for a token");

        let mut failed = Subscribing::default();
        failed.started();
        assert!(!failed.settled(false));
        assert!(failed.granted());

        let mut subscribed = Subscribing::default();
        subscribed.started();
        assert!(!subscribed.settled(true));
        assert!(!subscribed.granted());
    }

    #[test]
    fn a_grant_during_a_subscribe_retries_only_if_it_fails() {
        let mut failing = Subscribing::default();
        failing.started();
        assert!(!failing.granted(), "the running subscribe settles first");
        assert!(failing.settled(false), "the refused subscribe is retried");
        failing.started();
        assert!(!failing.settled(false), "a retry is made once");

        let mut succeeding = Subscribing::default();
        succeeding.started();
        assert!(!succeeding.granted());
        assert!(!succeeding.settled(true), "a subscription needs no retry");
    }

    #[test]
    fn a_retry_waits_for_the_last_subscribe_in_flight() {
        let mut queued = Subscribing::default();
        queued.started();
        queued.started();
        assert!(!queued.granted());
        assert!(!queued.settled(false), "another subscribe is still running");
        assert!(queued.settled(false), "the last one to fail retries");
    }
}
