//! The Apple entries the `#[pushups::rendering_handler]` expansion exports, which drive the
//! rendering service of one native service instance with the adapter's completion thunk.

use std::ffi::c_void;
use std::time::{Duration, Instant};

use crate::__private::ios::{Field, Finish};
use crate::rendering::{
    CompletionCallback, RenderingOutcome, RenderingService, Settled, mapped_fields,
};
use crate::{Error, Message, Rendering, RenderingContext, Suppression};

/// How long a render may run before the alert is left as sent, inside Apple's 30 seconds.
const RENDERING_BOUND: Duration = Duration::from_secs(25);

/// Nanoseconds per second, for capping the adapter's measured budget.
const NANOSECONDS_PER_SECOND: u64 = 1_000_000_000;

/// Creates the rendering service one native service instance owns, the handle `receive`,
/// `expired` and `deallocated` take, under the policy the extension's `Info.plist` declares.
///
/// # Safety
///
/// The handle is released exactly once, by `deallocated`.
pub(crate) fn create(filtering_declared: bool) -> *mut c_void {
    #[cfg(feature = "encryption")]
    super::keychain::install();
    let suppression = if filtering_declared {
        Suppression::Declared
    } else {
        Suppression::Undeclared
    };
    Box::into_raw(Box::new(RenderingService::new(suppression))).cast()
}

/// Starts the request for the alert the adapter received, superseding a pending one, and
/// runs the render within the budget the adapter measured from the alert's entry.
///
/// The render sees the envelope's plaintext when the adapter carried a `pushups` field and it
/// opens, the `userInfo` JSON otherwise, and a field that does not open settles `Failed`
/// without running the render, so the original alert stands.
///
/// `service` is a live, aligned handle from `create` that cannot be destroyed during this call. `user_info` and `pushups` point to readable fields of their declared lengths, the `pushups` pointer null when its length is zero. `finish` may run on any thread and owns the retained `token` until its single call.
pub(crate) unsafe fn receive(
    service: *const c_void,
    user_info: &Field,
    pushups: &Field,
    budget_nanos: u64,
    finish: Finish,
    token: *mut c_void,
    handler: fn(RenderingContext, Message) -> Result<Rendering, Error>,
) {
    // The deadline anchors before the payload copy, so the copy counts against the budget.
    let deadline = Instant::now()
        + Duration::from_nanos(
            budget_nanos.min(RENDERING_BOUND.as_secs() * NANOSECONDS_PER_SECOND),
        );
    // SAFETY: the caller passed a live handle and live fields of their declared lengths.
    unsafe {
        let user_info = std::slice::from_raw_parts(user_info.pointer, user_info.length);
        match payload(user_info, pushups.pointer, pushups.length) {
            Alert::Data(payload) => {
                let service = &*service.cast::<RenderingService>();
                service.begin(
                    deadline,
                    finish_callback(finish, SendToken(token)),
                    handler,
                    mapped_fields,
                    Message {
                        payload,
                        started_app: false,
                    },
                );
            }
            #[cfg(feature = "encryption")]
            Alert::Failed => {
                settle(finish, SendToken(token), RenderingOutcome::Failed, None);
            }
        }
    }
}

/// The alert's payload once its envelope is opened, the `userInfo` JSON when there is no
/// envelope, or the failure when the envelope could not be opened.
enum Alert {
    /// The payload the render sees, the envelope's plaintext or the `userInfo` JSON.
    Data(Vec<u8>),
    /// The envelope's field could not be opened, so the original alert stands.
    #[cfg(feature = "encryption")]
    Failed,
}

/// The alert's payload, opening its `pushups` field with the device's keys when it has one.
fn payload(user_info: &[u8], pushups: *const u8, pushups_len: usize) -> Alert {
    #[cfg(feature = "encryption")]
    {
        if pushups.is_null() || pushups_len == 0 {
            return Alert::Data(user_info.to_vec());
        }
        // SAFETY: the adapter passed a live field of `pushups_len` bytes.
        let field = unsafe { std::slice::from_raw_parts(pushups, pushups_len) };
        let Ok(field) = core::str::from_utf8(field) else {
            return Alert::Failed;
        };
        let Some(store) = crate::key_store::installed() else {
            return Alert::Failed;
        };
        match crate::envelope::open(store, field) {
            Ok(payload) => Alert::Data(payload),
            Err(_) => Alert::Failed,
        }
    }
    #[cfg(not(feature = "encryption"))]
    {
        let _ = (pushups, pushups_len);
        Alert::Data(user_info.to_vec())
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
    Box::new(move |outcome, content, _context| settle(finish, token, outcome, content.as_ref()))
}

/// Settles the adapter's thunk with `outcome` and the settled `content`'s fields.
fn settle(finish: Finish, token: SendToken, outcome: RenderingOutcome, content: Option<&Settled>) {
    let (title, body, tag, data, navigate) = match content {
        Some(Settled::Fields(fields)) => (
            Some(fields.title.as_str()),
            fields.body.as_deref(),
            fields.tag.as_deref(),
            fields.data.as_deref(),
            fields.navigate.as_deref(),
        ),
        _ => (None, None, None, None, None),
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
}

/// The pointer and length of a settled field for the adapter's thunk, or null and zero.
#[must_use]
fn field_bytes(value: Option<&str>) -> (*const u8, usize) {
    match value {
        Some(value) => (value.as_ptr(), value.len()),
        None => (std::ptr::null(), 0),
    }
}

/// The code the settled outcome takes on the wire. `Rendered` applies the fields,
/// `SuppressionRequested` completes with empty content, and the other codes settle the
/// original alert, which the marker distinguishes.
#[must_use]
fn outcome_code(outcome: RenderingOutcome) -> i32 {
    match outcome {
        RenderingOutcome::Rendered => 0,
        RenderingOutcome::Failed => 1,
        RenderingOutcome::Panicked => 2,
        RenderingOutcome::Expired => 3,
        RenderingOutcome::WorkerUnavailable => 4,
        RenderingOutcome::Superseded => 5,
        RenderingOutcome::SuppressionRequested => 6,
        RenderingOutcome::SuppressionUndeclared => 7,
    }
}
