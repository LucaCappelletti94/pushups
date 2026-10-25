//! Bounded notification rendering and shared request completion.

#[cfg(any(
    target_os = "ios",
    target_os = "macos",
    target_os = "android",
    target_os = "linux",
    target_os = "windows",
    test
))]
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
#[cfg(any(
    target_os = "ios",
    target_os = "macos",
    target_os = "android",
    target_os = "linux",
    target_os = "windows",
    test
))]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use parking_lot::Mutex;

use crate::Notification;
#[cfg(any(
    target_os = "ios",
    target_os = "macos",
    target_os = "android",
    target_os = "linux",
    target_os = "windows",
    test
))]
use crate::{Error, Message};

/// What a [`rendering_handler`](crate::rendering_handler) callback asks the system to present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rendering {
    /// Present this notification, replacing the original alert on Apple and shown through
    /// the platform's `Notification::show` on Android, Linux and Windows.
    Show(Notification),
    /// Present no alert, which on Apple requires a declared [`Suppression`] policy to
    /// suppress the original alert, while on Android, Linux and Windows it simply shows
    /// nothing.
    NoAlert,
}

impl From<Notification> for Rendering {
    fn from(notification: Notification) -> Self {
        Self::Show(notification)
    }
}

/// Whether a render may suppress its alert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suppression {
    /// No declaration, so [`Rendering::NoAlert`] keeps the original alert, which only an
    /// Apple extension that does not declare the filtering policy reaches.
    Undeclared,
    /// The alert may be suppressed, which on Apple completes [`Rendering::NoAlert`] with
    /// empty content under `PushupsNotificationFiltering`, an assertion Apple cannot check
    /// at runtime.
    Declared,
}

/// What the system shows after the render settles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderingOutcome {
    /// The render's notification was presented, replacing the original alert on Apple and
    /// shown through the platform on Android, Linux and Windows.
    Rendered,
    /// The render asked for no alert under a declared policy, which shows nothing off Apple
    /// and sends empty content on Apple.
    SuppressionRequested,
    /// The render asked for no alert without a declared policy, and the original alert
    /// stands, which only an Apple extension that does not declare the policy reaches.
    SuppressionUndeclared,
    /// The render returned an error, or content the platform cannot show, such as an icon
    /// URL, and nothing is shown.
    Failed,
    /// The render unwound.
    Panicked,
    /// The bound or the platform's expiration passed, and the original alert stands on
    /// Apple.
    Expired,
    /// The worker could not start, and the original alert stands on Apple.
    WorkerUnavailable,
    /// A newer render replaced this one before it settled.
    Superseded,
}

#[cfg(any(target_os = "ios", target_os = "macos"))]
impl RenderingOutcome {
    /// The settled alert's `pushups.rendering-outcome` value.
    pub(crate) const fn marker(self) -> &'static str {
        match self {
            Self::Rendered => "rendered",
            Self::SuppressionRequested => "suppression-requested",
            Self::SuppressionUndeclared => "suppression-undeclared",
            Self::Failed => "failed",
            Self::Panicked => "panicked",
            Self::Expired => "expired",
            Self::WorkerUnavailable => "worker-unavailable",
            Self::Superseded => "superseded",
        }
    }
}

/// Owned request identity, deadline and cooperative cancellation for a notification render.
#[derive(Clone)]
pub struct RenderingContext {
    state: Arc<RequestState>,
}

impl RenderingContext {
    /// The identifier of this request, fresh for every alert even when APNs reuses one.
    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.state.id
    }

    /// The monotonic deadline of the render's total bound.
    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.state.deadline
    }

    /// Whether the render should stop, true at the deadline and once the request settled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire) || Instant::now() >= self.state.deadline
    }

    /// Whether the render may suppress its alert, declared off Apple where the service
    /// decides for the whole process.
    #[must_use]
    pub fn suppression(&self) -> Suppression {
        self.state.suppression
    }

    /// The settled outcome, or `None` while the render runs.
    #[must_use]
    pub fn outcome(&self) -> Option<RenderingOutcome> {
        self.state.completion.lock().outcome
    }
}

/// The fields the alert's user info carries on Apple, the mobile-supported fields of the
/// `Notification` the render returned.
#[cfg(any(target_os = "ios", target_os = "macos", test))]
#[derive(Debug, PartialEq)]
pub(crate) struct RenderedFields {
    pub(crate) title: String,
    pub(crate) body: Option<String>,
    pub(crate) tag: Option<String>,
    pub(crate) data: Option<String>,
    pub(crate) navigate: Option<String>,
}

/// The content a settled render hands to the platform, which presents it or not by its
/// outcome.
#[cfg(any(
    target_os = "ios",
    target_os = "macos",
    target_os = "android",
    target_os = "linux",
    target_os = "windows",
    test
))]
#[derive(Debug, PartialEq)]
pub(crate) enum Settled {
    /// The alert's user info fields, the content Apple presents.
    #[cfg(any(target_os = "ios", target_os = "macos", test))]
    Fields(RenderedFields),
    /// The notification the render returned, whole, the content the platform shows.
    #[cfg(any(
        target_os = "android",
        target_os = "linux",
        target_os = "windows",
        test
    ))]
    Notification(Notification),
}

#[cfg(any(
    target_os = "ios",
    target_os = "macos",
    target_os = "android",
    target_os = "linux",
    target_os = "windows",
    test
))]
pub(crate) type CompletionCallback =
    Box<dyn FnOnce(RenderingOutcome, Option<Settled>, RenderingContext) + Send>;

/// The content Apple presents, the fields the platform can show, or `None` when the
/// notification carries an icon URL, which has no Apple mapping.
#[cfg(any(target_os = "ios", target_os = "macos", test))]
pub(crate) fn mapped_fields(notification: Notification) -> Option<Settled> {
    if notification.icon.is_some() {
        return None;
    }
    Some(Settled::Fields(RenderedFields {
        title: notification.title,
        body: notification.body,
        tag: notification.tag,
        data: notification.data,
        navigate: notification.navigate,
    }))
}

/// The content Android presents, the notification whole, or `None` for the icon URL its
/// presentation refuses.
#[cfg(any(target_os = "android", test))]
pub(crate) fn presentable(notification: Notification) -> Option<Settled> {
    notification
        .icon
        .is_none()
        .then_some(Settled::Notification(notification))
}

/// The content Linux and Windows present, the notification whole.
#[cfg(any(target_os = "linux", target_os = "windows", test))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the content functions settle through one signature that can refuse content"
)]
pub(crate) fn whole(notification: Notification) -> Option<Settled> {
    Some(Settled::Notification(notification))
}

/// The rendering service of one rendering instance, the one its platform entries drive, the
/// Apple extension or the process-wide handler on Android, Linux and Windows.
#[cfg(any(
    target_os = "ios",
    target_os = "macos",
    target_os = "android",
    target_os = "linux",
    target_os = "windows",
    test
))]
pub(crate) struct RenderingService {
    current: Mutex<Option<Arc<RequestState>>>,
    suppression: Suppression,
}

/// The process-wide generation of rendering requests, keeping request identities fresh
/// across service instances.
#[cfg(any(
    target_os = "ios",
    target_os = "macos",
    target_os = "android",
    target_os = "linux",
    target_os = "windows",
    test
))]
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(0);

#[cfg(any(
    target_os = "ios",
    target_os = "macos",
    target_os = "android",
    target_os = "linux",
    target_os = "windows",
    test
))]
impl RenderingService {
    /// The service with no request yet, under the declared `suppression`.
    #[must_use]
    pub(crate) const fn new(suppression: Suppression) -> Self {
        Self {
            current: Mutex::new(None),
            suppression,
        }
    }

    /// Starts the request for `message` under `deadline`, superseding a pending one, and
    /// returns the context the render runs with. `content` maps the `Show` notification to
    /// the content the platform presents, or `None` for content the platform cannot show,
    /// which settles `Failed`.
    #[cfg(any(
        target_os = "ios",
        target_os = "macos",
        target_os = "android",
        target_os = "linux",
        target_os = "windows",
        test
    ))]
    pub(crate) fn begin(
        &self,
        deadline: Instant,
        callback: CompletionCallback,
        handler: fn(RenderingContext, Message) -> Result<Rendering, Error>,
        content: fn(Notification) -> Option<Settled>,
        message: Message,
    ) -> RenderingContext {
        let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
        let state = Arc::new(RequestState {
            id: format!("pushups-render-{generation}"),
            deadline,
            suppression: self.suppression,
            cancelled: AtomicBool::new(false),
            completion: Mutex::new(Completion {
                outcome: None,
                callback: Some(callback),
            }),
        });
        let old = self.current.lock().replace(Arc::clone(&state));
        if let Some(old) = old {
            // Outside the service lock, since the settled request's callback may reenter begin.
            old.settle(RenderingOutcome::Superseded, None);
        }
        // The budget is already spent, so settle the original alert without running the
        // render.
        if Instant::now() >= deadline {
            state.settle(RenderingOutcome::Expired, None);
            return state.context();
        }
        let watchdog = Arc::clone(&state);
        let watchdog_thread = std::thread::Builder::new()
            .name("pushups notification deadline".to_owned())
            .spawn(move || {
                std::thread::sleep(watchdog.deadline.saturating_duration_since(Instant::now()));
                watchdog.settle(RenderingOutcome::Expired, None);
            });
        // A watchdog that cannot start leaves no deadline enforcement, so the request settles
        // unavailable instead of running past its bound.
        if watchdog_thread.is_err() {
            state.settle(RenderingOutcome::WorkerUnavailable, None);
            return state.context();
        }
        let worker_state = Arc::clone(&state);
        let worker = std::thread::Builder::new()
            .name("pushups rendering worker".to_owned())
            .spawn(move || {
                let render_context = worker_state.context();
                let result =
                    panic::catch_unwind(AssertUnwindSafe(|| handler(render_context, message)));
                let (outcome, settled) = if Instant::now() >= worker_state.deadline {
                    (RenderingOutcome::Expired, None)
                } else {
                    match result {
                        Ok(Ok(Rendering::Show(notification))) => match content(notification) {
                            Some(settled) => (RenderingOutcome::Rendered, Some(settled)),
                            None => (RenderingOutcome::Failed, None),
                        },
                        Ok(Ok(Rendering::NoAlert)) => match worker_state.suppression {
                            Suppression::Declared => (RenderingOutcome::SuppressionRequested, None),
                            Suppression::Undeclared => {
                                (RenderingOutcome::SuppressionUndeclared, None)
                            }
                        },
                        Ok(Err(_)) => (RenderingOutcome::Failed, None),
                        Err(_) => (RenderingOutcome::Panicked, None),
                    }
                };
                worker_state.settle(outcome, settled);
            });
        if worker.is_err() {
            state.settle(RenderingOutcome::WorkerUnavailable, None);
        }
        state.context()
    }

    /// The platform's expiration, which settles the pending request on the original alert.
    #[cfg(any(target_os = "ios", target_os = "macos", test))]
    pub(crate) fn expire(&self) {
        let current = self.current.lock().take();
        if let Some(current) = current {
            current.settle(RenderingOutcome::Expired, None);
        }
    }

    /// The service instance deallocated, which supersedes the pending request before the
    /// deadline and expires it after.
    #[cfg(any(target_os = "ios", target_os = "macos", test))]
    pub(crate) fn deallocate(&self) {
        let current = self.current.lock().take();
        if let Some(current) = current {
            current.settle(RenderingOutcome::Superseded, None);
        }
    }
}

/// The shared completion state of one request, the one behind the context of that request.
pub(crate) struct RequestState {
    id: String,
    deadline: Instant,
    suppression: Suppression,
    cancelled: AtomicBool,
    completion: Mutex<Completion>,
}

/// The one-shot completion of a request, decided and delivered exactly once.
pub(crate) struct Completion {
    outcome: Option<RenderingOutcome>,
    #[cfg(any(
        target_os = "ios",
        target_os = "macos",
        target_os = "android",
        target_os = "linux",
        target_os = "windows",
        test
    ))]
    callback: Option<CompletionCallback>,
}

impl RequestState {
    /// Settles the request with `outcome` and delivers it to the completion callback exactly
    /// once, outside the completion lock, so the callback may read the outcome or reenter the
    /// service. At the deadline, the selection normalizes to `Expired` under the arbitration
    /// lock, so a worker that crossed it while mapping or waiting cannot commit a late
    /// `Rendered`.
    #[cfg(any(
        target_os = "ios",
        target_os = "macos",
        target_os = "android",
        target_os = "linux",
        target_os = "windows",
        test
    ))]
    fn settle(self: &Arc<Self>, outcome: RenderingOutcome, content: Option<Settled>) {
        let (outcome, callback) = {
            let mut completion = self.completion.lock();
            if completion.outcome.is_some() {
                return;
            }
            let outcome = if Instant::now() >= self.deadline {
                RenderingOutcome::Expired
            } else {
                outcome
            };
            self.cancelled.store(true, Ordering::Release);
            completion.outcome = Some(outcome);
            (outcome, completion.callback.take())
        };
        let content = if outcome == RenderingOutcome::Expired {
            None
        } else {
            content
        };
        if let Some(callback) = callback {
            callback(outcome, content, self.context());
        }
    }

    /// The context this request's render runs with.
    #[cfg(any(
        target_os = "ios",
        target_os = "macos",
        target_os = "android",
        target_os = "linux",
        target_os = "windows",
        test
    ))]
    fn context(self: &Arc<Self>) -> RenderingContext {
        RenderingContext {
            state: Arc::clone(self),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Error, Message, Notification};
    use std::time::Duration;

    static MESSAGES: Mutex<Vec<(Vec<u8>, bool)>> = Mutex::new(Vec::new());

    #[derive(Default)]
    struct Recorded {
        count: usize,
        outcome: Option<RenderingOutcome>,
        content: Option<Settled>,
    }

    /// Waits until `holds` or the monotonic deadline passes, panicking on the deadline.
    fn wait_for(holds: impl Fn() -> bool, limit: Duration) {
        let deadline = Instant::now() + limit;
        while !holds() {
            assert!(Instant::now() < deadline, "the wait passed its deadline");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// The deadline `ms` milliseconds from now, for the tests' `begin` calls.
    fn deadline_in(ms: u64) -> Instant {
        Instant::now() + Duration::from_millis(ms)
    }

    /// The request every scenario starts under the Apple content mapping and a message the
    /// app had not started for.
    fn begin(
        service: &RenderingService,
        deadline: Instant,
        callback: CompletionCallback,
        payload: Vec<u8>,
    ) -> RenderingContext {
        service.begin(
            deadline,
            callback,
            test_handler,
            mapped_fields,
            Message {
                payload,
                started_app: false,
            },
        )
    }

    fn callback(recorded: &Arc<Mutex<Recorded>>) -> CompletionCallback {
        let recorded = Arc::clone(recorded);
        Box::new(move |outcome, content, _context| {
            let mut recorded = recorded.lock();
            recorded.count += 1;
            recorded.outcome = Some(outcome);
            recorded.content = content;
        })
    }

    fn rendered() -> Notification {
        Notification::new("Rendered")
            .body("Decrypted body")
            .tag("thread-1")
            .data("tap-data")
            .navigate("https://example.com")
    }

    /// The consumer render every scenario drives. A payload is a kind, optionally followed
    /// by `:sleep-<ms>`, which the render sleeps before answering.
    fn test_handler(_context: RenderingContext, message: Message) -> Result<Rendering, Error> {
        let Message {
            payload,
            started_app,
        } = message;
        let text = std::str::from_utf8(&payload).expect("the tests pass ascii payloads");
        let (kind, rest) = match text.split_once(':') {
            Some((kind, rest)) => (kind, rest),
            None => (text, ""),
        };
        let sleep = rest
            .strip_prefix("sleep-")
            .and_then(|ms| ms.parse::<u64>().ok())
            .unwrap_or(0);
        if sleep > 0 {
            std::thread::sleep(Duration::from_millis(sleep));
        }
        match kind {
            "error" => Err(Error::Platform(
                "the test render refused the payload".to_owned(),
            )),
            "panic" => panic!("the test render unwinds"),
            "icon" => Ok(Notification::new("Icon")
                .body("Icon body")
                .icon("https://example.com/icon.png")
                .into()),
            "no-alert" => Ok(Rendering::NoAlert),
            "record" => {
                MESSAGES.lock().push((payload, started_app));
                Ok(rendered().into())
            }
            _ => Ok(rendered().into()),
        }
    }

    #[test]
    fn the_context_keeps_its_identity_and_deadline_after_completion() {
        let service = RenderingService::new(Suppression::Undeclared);
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let context = begin(
            &service,
            deadline_in(5_000),
            callback(&recorded),
            b"render".to_vec(),
        );
        let deadline = context.deadline();
        wait_for(
            || context.outcome() == Some(RenderingOutcome::Rendered),
            Duration::from_secs(5),
        );
        // The accessors keep working after completion.
        assert_eq!(context.outcome(), Some(RenderingOutcome::Rendered));
        assert_eq!(context.deadline(), deadline);
    }

    #[test]
    fn a_transformed_alert_completes_rendered_with_its_mapped_fields() {
        let service = RenderingService::new(Suppression::Undeclared);
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let context = begin(
            &service,
            deadline_in(5_000),
            callback(&recorded),
            b"render".to_vec(),
        );
        wait_for(
            || recorded.lock().count == 1 && context.outcome() == Some(RenderingOutcome::Rendered),
            Duration::from_secs(5),
        );
        let recorded = recorded.lock();
        assert_eq!(recorded.outcome, Some(RenderingOutcome::Rendered));
        let settled = recorded.content.as_ref().expect("the render's content");
        let Settled::Fields(fields) = settled else {
            panic!("the content is not the mapped fields: {settled:?}")
        };
        assert_eq!(fields.title, "Rendered");
        assert_eq!(fields.body.as_deref(), Some("Decrypted body"));
        assert_eq!(fields.tag.as_deref(), Some("thread-1"));
        assert_eq!(fields.data.as_deref(), Some("tap-data"));
        assert_eq!(fields.navigate.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn a_returned_error_completes_failed_on_the_fallback() {
        let service = RenderingService::new(Suppression::Undeclared);
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let context = begin(
            &service,
            deadline_in(5_000),
            callback(&recorded),
            b"error".to_vec(),
        );
        wait_for(
            || recorded.lock().count == 1 && context.outcome() == Some(RenderingOutcome::Failed),
            Duration::from_secs(5),
        );
        assert!(
            recorded.lock().content.is_none(),
            "the fallback keeps no content"
        );
    }

    #[test]
    fn an_unwinding_render_completes_panicked() {
        let service = RenderingService::new(Suppression::Undeclared);
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let context = begin(
            &service,
            deadline_in(5_000),
            callback(&recorded),
            b"panic".to_vec(),
        );
        wait_for(
            || recorded.lock().count == 1 && context.outcome() == Some(RenderingOutcome::Panicked),
            Duration::from_secs(5),
        );
        assert!(
            recorded.lock().content.is_none(),
            "the fallback keeps no content"
        );
    }

    #[test]
    fn an_icon_url_completes_failed_for_the_fallback() {
        let service = RenderingService::new(Suppression::Undeclared);
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let context = begin(
            &service,
            deadline_in(5_000),
            callback(&recorded),
            b"icon".to_vec(),
        );
        wait_for(
            || recorded.lock().count == 1 && context.outcome() == Some(RenderingOutcome::Failed),
            Duration::from_secs(5),
        );
        assert!(
            recorded.lock().content.is_none(),
            "the fallback keeps no content"
        );
    }

    #[test]
    fn the_payload_is_the_original_user_info_and_the_app_did_not_start() {
        let service = RenderingService::new(Suppression::Undeclared);
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let payload = b"record:the-original-user-info".to_vec();
        let context = begin(
            &service,
            deadline_in(5_000),
            callback(&recorded),
            payload.clone(),
        );
        wait_for(
            || context.outcome() == Some(RenderingOutcome::Rendered),
            Duration::from_secs(5),
        );
        let messages = MESSAGES.lock();
        let entry = messages
            .iter()
            .find(|(entry_payload, _)| *entry_payload == payload)
            .expect("the render recorded the message");
        assert_eq!(&entry.0, &payload);
        assert!(!entry.1, "the extension does not start the app");
    }

    #[test]
    fn a_started_app_message_reaches_the_handler() {
        let service = RenderingService::new(Suppression::Undeclared);
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let payload = b"record:started-app".to_vec();
        service.begin(
            deadline_in(5_000),
            callback(&recorded),
            test_handler,
            whole,
            Message {
                payload: payload.clone(),
                started_app: true,
            },
        );
        wait_for(|| recorded.lock().count == 1, Duration::from_secs(5));
        let messages = MESSAGES.lock();
        let entry = messages
            .iter()
            .find(|(entry_payload, _)| *entry_payload == payload)
            .expect("the render recorded the message");
        assert!(entry.1, "the started flag reaches the handler");
    }

    #[test]
    fn the_whole_notification_reaches_the_platform_content() {
        let service = RenderingService::new(Suppression::Declared);
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let context = service.begin(
            deadline_in(5_000),
            callback(&recorded),
            test_handler,
            whole,
            Message {
                payload: b"render".to_vec(),
                started_app: false,
            },
        );
        assert_eq!(context.suppression(), Suppression::Declared);
        wait_for(
            || recorded.lock().count == 1 && context.outcome() == Some(RenderingOutcome::Rendered),
            Duration::from_secs(5),
        );
        let recorded = recorded.lock();
        assert_eq!(recorded.outcome, Some(RenderingOutcome::Rendered));
        assert_eq!(recorded.content, Some(Settled::Notification(rendered())));
    }

    #[test]
    fn an_icon_url_settles_failed_for_the_platform_content() {
        let service = RenderingService::new(Suppression::Undeclared);
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        service.begin(
            deadline_in(5_000),
            callback(&recorded),
            test_handler,
            presentable,
            Message {
                payload: b"icon".to_vec(),
                started_app: false,
            },
        );
        wait_for(|| recorded.lock().count == 1, Duration::from_secs(5));
        let recorded = recorded.lock();
        assert_eq!(recorded.outcome, Some(RenderingOutcome::Failed));
        assert!(recorded.content.is_none());
    }

    #[test]
    fn a_worker_returning_after_the_deadline_completes_expired_once() {
        let service = RenderingService::new(Suppression::Undeclared);
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        begin(
            &service,
            deadline_in(60),
            callback(&recorded),
            b"render:sleep-150".to_vec(),
        );
        wait_for(|| recorded.lock().count >= 1, Duration::from_secs(5));
        {
            let recorded = recorded.lock();
            assert_eq!(recorded.count, 1);
            assert_eq!(recorded.outcome, Some(RenderingOutcome::Expired));
            assert!(recorded.content.is_none(), "the late result is discarded");
        }
        // The worker's late return cannot add a second completion.
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(recorded.lock().count, 1);
    }

    #[test]
    fn an_already_spent_budget_completes_expired_without_running_the_render() {
        let service = RenderingService::new(Suppression::Undeclared);
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let context = begin(
            &service,
            Instant::now(),
            callback(&recorded),
            b"record:spent".to_vec(),
        );
        wait_for(
            || context.outcome() == Some(RenderingOutcome::Expired),
            Duration::from_secs(5),
        );
        let recorded = recorded.lock();
        assert_eq!(recorded.count, 1);
        assert_eq!(recorded.outcome, Some(RenderingOutcome::Expired));
        assert!(
            recorded.content.is_none(),
            "the spent budget runs no render"
        );
        assert!(
            MESSAGES
                .lock()
                .iter()
                .all(|(payload, _)| *payload != b"record:spent"),
            "the spent budget runs no render"
        );
    }

    #[test]
    fn an_error_returning_after_the_deadline_completes_expired() {
        let service = RenderingService::new(Suppression::Undeclared);
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        begin(
            &service,
            deadline_in(60),
            callback(&recorded),
            b"error:sleep-150".to_vec(),
        );
        wait_for(|| recorded.lock().count >= 1, Duration::from_secs(5));
        let recorded = recorded.lock();
        assert_eq!(recorded.count, 1);
        assert_eq!(recorded.outcome, Some(RenderingOutcome::Expired));
    }

    #[test]
    fn the_deadline_cancels_the_context_before_the_timer_delivers() {
        let service = RenderingService::new(Suppression::Undeclared);
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let context = begin(
            &service,
            deadline_in(80),
            callback(&recorded),
            b"render:sleep-200".to_vec(),
        );
        assert!(!context.is_cancelled(), "the deadline has not passed");
        std::thread::sleep(Duration::from_millis(120));
        assert!(context.is_cancelled());
        wait_for(|| recorded.lock().count >= 1, Duration::from_secs(5));
        assert_eq!(recorded.lock().outcome, Some(RenderingOutcome::Expired));
    }

    #[test]
    fn a_newer_alert_supersedes_the_pending_one() {
        let service = RenderingService::new(Suppression::Undeclared);
        let first = Arc::new(Mutex::new(Recorded::default()));
        let second = Arc::new(Mutex::new(Recorded::default()));
        let context_first = begin(
            &service,
            deadline_in(5_000),
            callback(&first),
            b"render:sleep-300".to_vec(),
        );
        let context_second = begin(
            &service,
            deadline_in(5_000),
            callback(&second),
            b"render".to_vec(),
        );
        wait_for(
            || context_first.outcome() == Some(RenderingOutcome::Superseded),
            Duration::from_secs(5),
        );
        wait_for(
            || context_second.outcome() == Some(RenderingOutcome::Rendered),
            Duration::from_secs(5),
        );
        assert!(context_first.is_cancelled());
        assert!(
            first.lock().content.is_none(),
            "the fallback keeps no content"
        );
    }

    #[test]
    fn a_supersession_after_the_deadline_completes_expired() {
        let service = RenderingService::new(Suppression::Undeclared);
        let first = Arc::new(Mutex::new(Recorded::default()));
        let second = Arc::new(Mutex::new(Recorded::default()));
        let context_first = begin(
            &service,
            deadline_in(60),
            callback(&first),
            b"render:sleep-300".to_vec(),
        );
        std::thread::sleep(Duration::from_millis(120));
        let context_second = begin(
            &service,
            deadline_in(5_000),
            callback(&second),
            b"render".to_vec(),
        );
        wait_for(
            || context_first.outcome() == Some(RenderingOutcome::Expired),
            Duration::from_secs(5),
        );
        wait_for(
            || context_second.outcome() == Some(RenderingOutcome::Rendered),
            Duration::from_secs(5),
        );
    }

    #[test]
    fn a_stale_timer_cannot_complete_the_newer_request() {
        let service = RenderingService::new(Suppression::Undeclared);
        let first = Arc::new(Mutex::new(Recorded::default()));
        let second = Arc::new(Mutex::new(Recorded::default()));
        let context_first = begin(
            &service,
            deadline_in(80),
            callback(&first),
            b"render:sleep-400".to_vec(),
        );
        let context_second = begin(
            &service,
            deadline_in(5_000),
            callback(&second),
            b"render".to_vec(),
        );
        wait_for(
            || {
                second.lock().count == 1
                    && context_second.outcome() == Some(RenderingOutcome::Rendered)
            },
            Duration::from_secs(5),
        );
        std::thread::sleep(Duration::from_millis(120));
        // The first request's timer fired stale after the supersession.
        assert_eq!(context_first.outcome(), Some(RenderingOutcome::Superseded));
        assert_eq!(first.lock().count, 1);
        assert_eq!(
            second.lock().count,
            1,
            "the stale timer left the newer one alone"
        );
        assert_eq!(context_second.outcome(), Some(RenderingOutcome::Rendered));
    }

    #[test]
    fn the_platform_expiration_completes_expired_and_drops_the_late_worker() {
        let service = RenderingService::new(Suppression::Undeclared);
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let context = begin(
            &service,
            deadline_in(5_000),
            callback(&recorded),
            b"render:sleep-300".to_vec(),
        );
        std::thread::sleep(Duration::from_millis(50));
        service.expire();
        assert_eq!(context.outcome(), Some(RenderingOutcome::Expired));
        wait_for(|| recorded.lock().count >= 1, Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(400));
        let recorded = recorded.lock();
        assert_eq!(recorded.count, 1, "the late worker adds nothing");
        assert_eq!(recorded.outcome, Some(RenderingOutcome::Expired));
    }

    #[test]
    fn a_settled_request_ignores_later_expirations_and_deallocations() {
        let service = RenderingService::new(Suppression::Undeclared);
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let context = begin(
            &service,
            deadline_in(5_000),
            callback(&recorded),
            b"render".to_vec(),
        );
        wait_for(
            || recorded.lock().count == 1 && context.outcome() == Some(RenderingOutcome::Rendered),
            Duration::from_secs(5),
        );
        service.expire();
        service.deallocate();
        assert_eq!(recorded.lock().count, 1, "completion is exactly once");
        assert_eq!(context.outcome(), Some(RenderingOutcome::Rendered));
    }

    #[test]
    fn the_deallocation_supersedes_the_pending_request() {
        let service = RenderingService::new(Suppression::Undeclared);
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let context = begin(
            &service,
            deadline_in(5_000),
            callback(&recorded),
            b"render:sleep-300".to_vec(),
        );
        std::thread::sleep(Duration::from_millis(50));
        service.deallocate();
        assert_eq!(context.outcome(), Some(RenderingOutcome::Superseded));
        wait_for(|| recorded.lock().count >= 1, Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(recorded.lock().count, 1, "the late worker adds nothing");
    }

    #[test]
    fn reused_payloads_keep_fresh_request_ids() {
        let service = RenderingService::new(Suppression::Undeclared);
        let payload = b"render".to_vec();
        let first = begin(
            &service,
            deadline_in(5_000),
            callback(&Arc::new(Mutex::new(Recorded::default()))),
            payload.clone(),
        );
        wait_for(
            || first.outcome() == Some(RenderingOutcome::Rendered),
            Duration::from_secs(5),
        );
        let second = begin(
            &service,
            deadline_in(5_000),
            callback(&Arc::new(Mutex::new(Recorded::default()))),
            payload,
        );
        wait_for(
            || second.outcome() == Some(RenderingOutcome::Rendered),
            Duration::from_secs(5),
        );
        assert_ne!(first.request_id(), second.request_id());
    }

    #[test]
    fn the_completion_runs_outside_the_lock_and_reenters_the_service() {
        let service = Arc::new(RenderingService::new(Suppression::Undeclared));
        let inner = Arc::clone(&service);
        let inner_recorded = Arc::new(Mutex::new(Recorded::default()));
        let inner_callback = callback(&inner_recorded);
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let callback_recorded = Arc::clone(&recorded);
        let context = service.begin(
            deadline_in(5_000),
            Box::new(move |outcome, content, context| {
                // The completion lock is not held here, so the outcome is readable and the
                // service can be reentered.
                assert_eq!(context.outcome(), Some(outcome));
                let reentered = inner.begin(
                    deadline_in(5_000),
                    inner_callback,
                    test_handler,
                    mapped_fields,
                    Message {
                        payload: b"render".to_vec(),
                        started_app: false,
                    },
                );
                wait_for(
                    || reentered.outcome() == Some(RenderingOutcome::Rendered),
                    Duration::from_secs(2),
                );
                let mut recorded = callback_recorded.lock();
                recorded.count += 1;
                recorded.outcome = Some(outcome);
                recorded.content = content;
            }),
            test_handler,
            mapped_fields,
            Message {
                payload: b"render".to_vec(),
                started_app: false,
            },
        );
        wait_for(
            || {
                recorded.lock().count == 1
                    && inner_recorded.lock().count == 1
                    && context.outcome() == Some(RenderingOutcome::Rendered)
            },
            Duration::from_secs(5),
        );
        assert_eq!(recorded.lock().count, 1);
        assert_eq!(
            inner_recorded.lock().count,
            1,
            "the reentered request settled too"
        );
    }

    #[test]
    fn settlement_after_the_deadline_discards_rendered_content() {
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let state = Arc::new(RequestState {
            id: "late-request".to_owned(),
            deadline: Instant::now(),
            suppression: Suppression::Undeclared,
            cancelled: AtomicBool::new(false),
            completion: Mutex::new(Completion {
                outcome: None,
                callback: Some(callback(&recorded)),
            }),
        });
        state.settle(RenderingOutcome::Rendered, mapped_fields(rendered()));
        assert_eq!(state.context().outcome(), Some(RenderingOutcome::Expired));
        assert!(state.context().is_cancelled());
        let result = recorded.lock();
        assert_eq!(result.outcome, Some(RenderingOutcome::Expired));
        assert!(result.content.is_none());
    }

    #[test]
    fn the_context_and_outcome_are_send_and_sync() {
        const _: fn() = || {
            fn assert_send_sync<T: Send + Sync>() {}
            assert_send_sync::<RenderingContext>();
            assert_send_sync::<RenderingOutcome>();
        };
    }

    #[test]
    fn no_alert_under_a_declared_policy_completes_suppression_requested() {
        let service = RenderingService::new(Suppression::Declared);
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let context = begin(
            &service,
            deadline_in(5_000),
            callback(&recorded),
            b"no-alert".to_vec(),
        );
        assert_eq!(context.suppression(), Suppression::Declared);
        wait_for(|| recorded.lock().count == 1, Duration::from_secs(5));
        let recorded = recorded.lock();
        assert_eq!(
            recorded.outcome,
            Some(RenderingOutcome::SuppressionRequested)
        );
        assert!(recorded.content.is_none());
    }

    #[test]
    fn no_alert_without_a_declared_policy_keeps_the_original_alert() {
        let service = RenderingService::new(Suppression::Undeclared);
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let context = begin(
            &service,
            deadline_in(5_000),
            callback(&recorded),
            b"no-alert".to_vec(),
        );
        assert_eq!(context.suppression(), Suppression::Undeclared);
        wait_for(|| recorded.lock().count == 1, Duration::from_secs(5));
        let recorded = recorded.lock();
        assert_eq!(
            recorded.outcome,
            Some(RenderingOutcome::SuppressionUndeclared)
        );
        assert!(recorded.content.is_none());
    }

    #[test]
    fn a_no_alert_after_the_deadline_completes_expired_once() {
        let service = RenderingService::new(Suppression::Declared);
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let context = begin(
            &service,
            deadline_in(50),
            callback(&recorded),
            b"no-alert:sleep-200".to_vec(),
        );
        wait_for(
            || context.outcome() == Some(RenderingOutcome::Expired),
            Duration::from_secs(5),
        );
        std::thread::sleep(Duration::from_millis(300));
        let recorded = recorded.lock();
        assert_eq!(recorded.count, 1);
        assert_eq!(recorded.outcome, Some(RenderingOutcome::Expired));
    }

    #[test]
    fn a_superseded_no_alert_cannot_settle_after_its_replacement() {
        let service = RenderingService::new(Suppression::Declared);
        let first = Arc::new(Mutex::new(Recorded::default()));
        let second = Arc::new(Mutex::new(Recorded::default()));
        let context_first = begin(
            &service,
            deadline_in(5_000),
            callback(&first),
            b"no-alert:sleep-200".to_vec(),
        );
        let context_second = begin(
            &service,
            deadline_in(5_000),
            callback(&second),
            b"render".to_vec(),
        );
        wait_for(
            || context_second.outcome() == Some(RenderingOutcome::Rendered),
            Duration::from_secs(5),
        );
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(context_first.outcome(), Some(RenderingOutcome::Superseded));
        assert_eq!(first.lock().count, 1);
    }
}
