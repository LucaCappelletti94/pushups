//! The Apple entries the `#[pushups::notification_service]` expansion exports, which drive the
//! rendering service of one native service instance with the adapter's completion thunk.

use std::ffi::c_void;
use std::time::{Duration, Instant};

use crate::__private::ios::Finish;
use crate::rendering::{CompletionCallback, RenderingOutcome, RenderingService};
use crate::{Error, Message, Notification, RenderingContext};

/// How long a transform may run before the alert is left as sent, inside Apple's 30 seconds.
const RENDERING_BOUND: Duration = Duration::from_secs(25);

/// Nanoseconds per second, for capping the adapter's measured budget.
const NANOSECONDS_PER_SECOND: u64 = 1_000_000_000;

/// Creates the rendering service one native service instance owns, the handle `receive`,
/// `expired` and `deallocated` take.
///
/// # Safety
///
/// The handle is released exactly once, by `deallocated`.
pub(crate) fn create() -> *mut c_void {
    Box::into_raw(Box::new(RenderingService::new())).cast()
}

/// Starts the request for the alert the adapter received, superseding a pending one, and
/// runs the transform within the budget the adapter measured from the alert's entry.
///
/// # Safety
///
/// `service` is a live, aligned handle from `create` that cannot be destroyed during this call. `user_info` points to `user_info_len` readable bytes. `finish` may run on any thread and owns the retained `token` until its single call.
pub(crate) unsafe fn receive(
    service: *const c_void,
    user_info: *const u8,
    user_info_len: usize,
    budget_nanos: u64,
    finish: Finish,
    token: *mut c_void,
    handler: fn(RenderingContext, Message) -> Result<Notification, Error>,
) {
    // The deadline anchors before the payload copy, so the copy counts against the budget.
    let deadline = Instant::now()
        + Duration::from_nanos(
            budget_nanos.min(RENDERING_BOUND.as_secs() * NANOSECONDS_PER_SECOND),
        );
    // SAFETY: the caller passed a live handle and a payload of `user_info_len` bytes.
    unsafe {
        let service = &*service.cast::<RenderingService>();
        let user_info = std::slice::from_raw_parts(user_info, user_info_len);
        service.begin(
            deadline,
            finish_callback(finish, SendToken(token)),
            handler,
            user_info.to_vec(),
        );
    }
}

/// The platform's `serviceExtensionTimeWillExpire` for the instance.
///
/// # Safety
///
/// `service` is a live handle from `create`.
pub(crate) unsafe fn expired(service: *const c_void) {
    // SAFETY: the caller passed a live handle.
    unsafe {
        let service = &*service.cast::<RenderingService>();
        service.expire();
    }
}

/// Settles the pending request and destroys the service handle.
///
/// # Safety
///
/// `service` is the live handle from `create`, exclusively released once with no concurrent calls.
pub(crate) unsafe fn deallocated(service: *mut c_void) {
    // SAFETY: the caller released the handle exactly once.
    let service = unsafe { Box::from_raw(service.cast::<RenderingService>()) };
    service.deallocate();
}

/// The adapter's completion box token, the one the machine sends to the settling thread.
struct SendToken(*mut c_void);

// SAFETY: `receive` transfers exclusive token access to a thunk callable from any thread.
unsafe impl Send for SendToken {}

impl SendToken {
    /// The token for the thunk call, consuming the wrapper.
    #[must_use]
    fn into_ptr(self) -> *mut c_void {
        self.0
    }
}

/// The completion callback that hands the settled result to the adapter's thunk.
fn finish_callback(finish: Finish, token: SendToken) -> CompletionCallback {
    Box::new(move |outcome, fields, _context| {
        let (title, body, tag, data, navigate) = match &fields {
            Some(fields) => (
                Some(fields.title.as_str()),
                fields.body.as_deref(),
                fields.tag.as_deref(),
                fields.data.as_deref(),
                fields.navigate.as_deref(),
            ),
            None => (None, None, None, None, None),
        };
        // SAFETY: the machine settles the request exactly once, and this is its one call, with
        // the strings alive for the call and `finish` the adapter's thunk.
        unsafe {
            let (title, title_len) = field_bytes(title);
            let (body, body_len) = field_bytes(body);
            let (tag, tag_len) = field_bytes(tag);
            let (data, data_len) = field_bytes(data);
            let (navigate, navigate_len) = field_bytes(navigate);
            finish(
                token.into_ptr(),
                outcome_code(outcome),
                outcome.marker().as_ptr(),
                outcome.marker().len(),
                title,
                title_len,
                body,
                body_len,
                tag,
                tag_len,
                data,
                data_len,
                navigate,
                navigate_len,
            );
        }
    })
}

/// The pointer and length of a settled field for the adapter's thunk, or null and zero.
#[must_use]
fn field_bytes(value: Option<&str>) -> (*const u8, usize) {
    match value {
        Some(value) => (value.as_ptr(), value.len()),
        None => (std::ptr::null(), 0),
    }
}

/// The code the settled outcome takes on the wire. `Rendered` applies the fields, and the
/// other codes settle the original alert, which the marker distinguishes.
#[must_use]
fn outcome_code(outcome: RenderingOutcome) -> i32 {
    match outcome {
        RenderingOutcome::Rendered => 0,
        RenderingOutcome::Failed => 1,
        RenderingOutcome::Panicked => 2,
        RenderingOutcome::Expired => 3,
        RenderingOutcome::WorkerUnavailable => 4,
        RenderingOutcome::Superseded => 5,
    }
}
