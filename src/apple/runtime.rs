//! The Objective-C side of the Apple backend: the delegates the crate installs or extends, the launch observer and the key-value observer of the notification center's delegate.
//!
//! An existing delegate is extended by an isa-swizzle, Firebase's technique. A subclass of its class, named with [`PREFIX`], adds the crate's methods, and the instance's class is set to it, so the toolkit's class stays untouched and the instance keeps its identity. Each added method calls the original class's implementation when there is one.

use std::ffi::{CStr, CString, c_void};
use std::ptr::NonNull;
#[cfg(target_os = "ios")]
use std::sync::Arc;

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, Bool, ClassBuilder, NSObject, Sel};
use objc2::{ClassType, MainThreadMarker, class, msg_send, sel};
use objc2_foundation::{
    NSData, NSDictionary, NSError, NSJSONSerialization, NSJSONWritingOptions,
    NSKeyValueObservingOptions, NSNotification, NSNotificationCenter, NSString,
};
use objc2_user_notifications::{UNNotificationPresentationOptions, UNUserNotificationCenter};
#[cfg(target_os = "ios")]
use parking_lot::Mutex;

use crate::started::Arrival;

/// Starts the name of every class the crate creates, so the crate recognises an object it already extended.
const PREFIX: &str = "Pushups";

/// The prefix of a subclass extending an existing delegate's class.
const SWIZZLED: &str = "PushupsSwizzled_";

/// `UIBackgroundFetchResult` values.
#[cfg(target_os = "ios")]
const FETCH_NEW_DATA: usize = 0;
#[cfg(target_os = "ios")]
const FETCH_NO_DATA: usize = 1;

/// Takes over the notification center's delegate now, and every delegate the app sets later.
pub(super) fn take_over_notification_delegate(_mtm: MainThreadMarker) {
    let center = UNUserNotificationCenter::currentNotificationCenter();
    adopt_notification_delegate(&center);
    let observer = new_object(delegate_observer_class());
    // SAFETY: the observer implements `observeValueForKeyPath:ofObject:change:context:` and lives for the process, so the center never messages a freed object.
    unsafe {
        let _: () = msg_send![
            &*center,
            addObserver: &*observer,
            forKeyPath: &*NSString::from_str("delegate"),
            options: NSKeyValueObservingOptions::New,
            context: std::ptr::null_mut::<c_void>(),
        ];
    }
    super::keep(observer);
}

/// Installs the crate's notification delegate when the center has none, and extends the one it has.
fn adopt_notification_delegate(center: &UNUserNotificationCenter) {
    // SAFETY: `delegate` returns the center's delegate or nil.
    let delegate: Option<Retained<AnyObject>> = unsafe { msg_send![center, delegate] };
    if let Some(delegate) = delegate {
        if extend(&delegate) {
            // The center caches which optional methods its delegate implements, so it reads them again.
            // SAFETY: setting the same delegate back, which the center holds weakly and the app keeps alive.
            unsafe {
                let _: () = msg_send![center, setDelegate: std::ptr::null::<AnyObject>()];
                let _: () = msg_send![center, setDelegate: &*delegate];
            }
        }
    } else {
        let own = new_object(own_notification_delegate_class());
        // SAFETY: the crate keeps its delegate alive for the process, since the center holds it weakly.
        let _: () = unsafe { msg_send![center, setDelegate: &*own] };
        super::keep(own);
    }
}

/// Watches for `didFinishLaunching` and `didBecomeActive`, and extends the app delegate at once if the launch already happened.
pub(super) fn observe_launch(mtm: MainThreadMarker) {
    if let Some(application) = shared_application(mtm) {
        // SAFETY: `delegate` returns the application's delegate or nil.
        let delegate: Option<Retained<AnyObject>> = unsafe { msg_send![&*application, delegate] };
        if delegate.is_some() {
            adopt_app_delegate(&application);
        }
    }
    observe(did_finish_launching_name(), on_launch);
    observe(did_become_active_name(), super::became_active);
}

/// Calls `action` on the main thread every time the notification `name` posts, for the life of the process.
fn observe(name: &NSString, action: fn()) {
    let block = RcBlock::new(move |_: NonNull<NSNotification>| action());
    // SAFETY: the application posts these notifications on the main thread, where the block runs, and the observer token lives for the process.
    let token = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(name),
            None,
            None,
            &block,
        )
    };
    std::mem::forget(token);
}

fn on_launch() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(application) = shared_application(mtm) else {
        return;
    };
    adopt_app_delegate(&application);
    super::launched(launched_in_background(&application));
}

/// Whether the system launched the app in the background, as it does for a push.
#[cfg(target_os = "ios")]
fn launched_in_background(application: &AnyObject) -> bool {
    /// `UIApplicationStateBackground`.
    const BACKGROUND: isize = 2;
    // SAFETY: `applicationState` is a getter of `UIApplication`, read on the main thread.
    let state: isize = unsafe { msg_send![application, applicationState] };
    state == BACKGROUND
}

/// macOS never launches an app for a push.
#[cfg(target_os = "macos")]
fn launched_in_background(_application: &AnyObject) -> bool {
    false
}

#[cfg(target_os = "ios")]
fn did_finish_launching_name() -> &'static NSString {
    // SAFETY: a static string of UIKit.
    unsafe { objc2_ui_kit::UIApplicationDidFinishLaunchingNotification }
}

#[cfg(target_os = "macos")]
fn did_finish_launching_name() -> &'static NSString {
    // SAFETY: a static string of AppKit.
    unsafe { objc2_app_kit::NSApplicationDidFinishLaunchingNotification }
}

#[cfg(target_os = "ios")]
fn did_become_active_name() -> &'static NSString {
    // SAFETY: a static string of UIKit.
    unsafe { objc2_ui_kit::UIApplicationDidBecomeActiveNotification }
}

#[cfg(target_os = "macos")]
fn did_become_active_name() -> &'static NSString {
    // SAFETY: a static string of AppKit.
    unsafe { objc2_app_kit::NSApplicationDidBecomeActiveNotification }
}

/// The application object, nil until `UIApplicationMain` runs on iOS.
#[cfg(target_os = "ios")]
fn shared_application(_mtm: MainThreadMarker) -> Option<Retained<AnyObject>> {
    // SAFETY: a class method returning the singleton or nil, on the main thread.
    unsafe { msg_send![class!(UIApplication), sharedApplication] }
}

/// The application object, nil until the toolkit creates it on macOS.
///
/// It reads the `NSApp` global, since `sharedApplication` would create a plain `NSApplication` before tao or winit create their subclass of it.
#[cfg(target_os = "macos")]
fn shared_application(_mtm: MainThreadMarker) -> Option<Retained<AnyObject>> {
    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {
        static mut NSApp: *mut AnyObject;
    }
    // SAFETY: AppKit writes `NSApp` on the main thread only, where this runs, and it holds the application or null.
    let application = unsafe { (&raw const NSApp).read() };
    // SAFETY: a live application object, retained here for the caller.
    unsafe { Retained::retain(application) }
}

/// Extends the application's delegate, or installs the crate's own when there is none.
fn adopt_app_delegate(application: &AnyObject) {
    // SAFETY: `delegate` returns the application's delegate or nil.
    let delegate: Option<Retained<AnyObject>> = unsafe { msg_send![application, delegate] };
    if let Some(delegate) = delegate {
        // UIKit caches whether the delegate implements the remote notification methods, so it reads them again, as Firebase does. AppKit registers its delegate as the observer of `didFinishLaunching`, which is posting now, so a reset there would drop the toolkit's own launch.
        #[cfg(target_os = "ios")]
        if extend(&delegate) {
            // SAFETY: setting the same delegate back. `UIApplicationMain` owns the delegate it created and releases it on the reset, so the crate keeps it alive from here on.
            unsafe {
                let _: () = msg_send![application, setDelegate: std::ptr::null::<AnyObject>()];
                let _: () = msg_send![application, setDelegate: &*delegate];
            }
            super::keep(delegate);
        }
        #[cfg(target_os = "macos")]
        extend(&delegate);
    } else {
        let own = new_object(own_app_delegate_class());
        // SAFETY: the crate keeps its delegate alive for the process, since the application holds it weakly.
        let _: () = unsafe { msg_send![application, setDelegate: &*own] };
        super::keep(own);
    }
}

pub(super) fn register_for_remote_notifications(mtm: MainThreadMarker) {
    if let Some(application) = shared_application(mtm) {
        // SAFETY: an instance method of `UIApplication` and `NSApplication`, on the main thread.
        let _: () = unsafe { msg_send![&*application, registerForRemoteNotifications] };
    }
}

fn new_object(class: &AnyClass) -> Retained<AnyObject> {
    // SAFETY: every class here descends from `NSObject`, whose `new` returns a retained instance.
    unsafe { msg_send![class, new] }
}

/// Whether `class` is one the crate created, or descends from one.
fn is_ours(class: &AnyClass) -> bool {
    std::iter::successors(Some(class), |class| class.superclass())
        .any(|class| class.name().to_bytes().starts_with(PREFIX.as_bytes()))
}

/// Sets `object`'s class to the crate's subclass of it. Returns `false` when the object was already extended.
fn extend(object: &AnyObject) -> bool {
    let original = object.class();
    if is_ours(original) {
        return false;
    }
    let Ok(name) = CString::new(format!("{SWIZZLED}{}", original.name().to_string_lossy())) else {
        return false;
    };
    let subclass = AnyClass::get(&name).unwrap_or_else(|| {
        let mut builder = ClassBuilder::new(&name, original)
            .expect("the class name is free, since `AnyClass::get` found none");
        add_push_methods(&mut builder);
        builder.register()
    });
    // SAFETY: the subclass adds methods and no instance variables, so the object's layout fits it.
    unsafe { AnyObject::set_class(object, subclass) };
    true
}

/// The crate's notification delegate, whose `willPresentNotification:` shows a foreground push as the system shows a background one.
const OWN_NOTIFICATION_DELEGATE: &CStr = c"PushupsNotificationDelegate";

fn own_notification_delegate_class() -> &'static AnyClass {
    own_class(
        OWN_NOTIFICATION_DELEGATE,
        c"UNUserNotificationCenterDelegate",
        |_| {},
    )
}

fn own_app_delegate_class() -> &'static AnyClass {
    #[cfg(target_os = "ios")]
    let protocol = c"UIApplicationDelegate";
    #[cfg(target_os = "macos")]
    let protocol = c"NSApplicationDelegate";
    own_class(c"PushupsAppDelegate", protocol, |_| {})
}

/// A class of the crate's own, an `NSObject` with the push methods, the extra methods `more` adds, and `protocol`.
fn own_class(
    name: &CStr,
    protocol: &CStr,
    more: impl FnOnce(&mut ClassBuilder),
) -> &'static AnyClass {
    if let Some(class) = AnyClass::get(name) {
        return class;
    }
    let mut builder = ClassBuilder::new(name, NSObject::class())
        .expect("the class name is free, since `AnyClass::get` found none");
    if let Some(protocol) = AnyProtocol::get(protocol) {
        builder.add_protocol(protocol);
    }
    add_push_methods(&mut builder);
    more(&mut builder);
    builder.register()
}

/// The class of the object watching the notification center's `delegate` property.
fn delegate_observer_class() -> &'static AnyClass {
    let name = c"PushupsDelegateObserver";
    if let Some(class) = AnyClass::get(name) {
        return class;
    }
    let mut builder = ClassBuilder::new(name, NSObject::class())
        .expect("the class name is free, since `AnyClass::get` found none");
    // SAFETY: the signature of `observeValueForKeyPath:ofObject:change:context:`.
    unsafe {
        builder.add_method(
            sel!(observeValueForKeyPath:ofObject:change:context:),
            delegate_changed as unsafe extern "C-unwind" fn(_, _, _, _, _, _),
        );
    }
    builder.register()
}

/// The methods the crate adds to every delegate it extends or creates.
fn add_push_methods(builder: &mut ClassBuilder) {
    // SAFETY: each function has the signature its selector has in UIKit, AppKit and UserNotifications.
    unsafe {
        builder.add_method(
            sel!(application:didRegisterForRemoteNotificationsWithDeviceToken:),
            did_register as unsafe extern "C-unwind" fn(_, _, _, _),
        );
        builder.add_method(
            sel!(application:didFailToRegisterForRemoteNotificationsWithError:),
            did_fail_to_register as unsafe extern "C-unwind" fn(_, _, _, _),
        );
        #[cfg(target_os = "ios")]
        builder.add_method(
            sel!(application:didReceiveRemoteNotification:fetchCompletionHandler:),
            did_receive_remote_notification as unsafe extern "C-unwind" fn(_, _, _, _, _),
        );
        #[cfg(target_os = "macos")]
        builder.add_method(
            sel!(application:didReceiveRemoteNotification:),
            did_receive_remote_notification as unsafe extern "C-unwind" fn(_, _, _, _),
        );
        builder.add_method(
            sel!(userNotificationCenter:willPresentNotification:withCompletionHandler:),
            will_present as unsafe extern "C-unwind" fn(_, _, _, _, _),
        );
        builder.add_method(
            sel!(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:),
            did_receive_response as unsafe extern "C-unwind" fn(_, _, _, _, _),
        );
    }
}

/// The class above the crate's own in `this`'s hierarchy, when it implements `cmd`, so the crate's method chains to it.
fn original_with(this: &AnyObject, cmd: Sel) -> Option<&'static AnyClass> {
    let ours = std::iter::successors(Some(this.class()), |class| class.superclass())
        .find(|class| class.name().to_bytes().starts_with(PREFIX.as_bytes()))?;
    ours.superclass()
        .filter(|original| original.instance_method(cmd).is_some())
}

/// The JSON encoding of a property-list dictionary such as a push's `userInfo`.
fn json(object: &AnyObject) -> Option<Vec<u8>> {
    // SAFETY: `isValidJSONObject` accepts any object and never throws.
    if !unsafe { NSJSONSerialization::isValidJSONObject(object) } {
        return None;
    }
    // SAFETY: `isValidJSONObject` confirmed the object converts, so this cannot throw.
    let data: Result<Retained<NSData>, Retained<NSError>> = unsafe {
        NSJSONSerialization::dataWithJSONObject_options_error(object, NSJSONWritingOptions::empty())
    };
    data.ok().map(|data| data.to_vec())
}

unsafe extern "C-unwind" fn did_register(
    this: &AnyObject,
    cmd: Sel,
    application: *mut AnyObject,
    token: *mut NSData,
) {
    if let Some(original) = original_with(this, cmd) {
        // SAFETY: the original class implements this method with this signature.
        unsafe {
            let _: () = msg_send![super(this, original), application: application, didRegisterForRemoteNotificationsWithDeviceToken: token];
        }
    }
    // SAFETY: the system passes an `NSData` or null, valid for this call.
    if let Some(token) = unsafe { token.as_ref() } {
        super::token(token.to_vec());
    }
}

unsafe extern "C-unwind" fn did_fail_to_register(
    this: &AnyObject,
    cmd: Sel,
    application: *mut AnyObject,
    error: *mut NSError,
) {
    if let Some(original) = original_with(this, cmd) {
        // SAFETY: the original class implements this method with this signature.
        unsafe {
            let _: () = msg_send![super(this, original), application: application, didFailToRegisterForRemoteNotificationsWithError: error];
        }
    }
    // SAFETY: the system passes an `NSError` or null, valid for this call.
    let description = unsafe { error.as_ref() }.map_or_else(
        || "APNs registration failed".to_owned(),
        |error| error.localizedDescription().to_string(),
    );
    super::registration_failed(description);
}

/// The completion handler of one iOS push, called once both the original method and the crate's background work are done, with the most useful result either reported.
#[cfg(target_os = "ios")]
struct FetchGate {
    remaining: u8,
    result: usize,
    completion: Option<RcBlock<dyn Fn(usize)>>,
}

/// Holds a value that crosses threads only as Apple's completion handlers do.
#[cfg(target_os = "ios")]
struct AnyThread<T>(T);

// SAFETY: the only content is a `FetchGate` holding a completion handler, which UIKit allows calling from any thread, and every access goes through the `Mutex`.
#[cfg(target_os = "ios")]
unsafe impl<T> Send for AnyThread<T> {}
// SAFETY: as for `Send`, every access goes through the `Mutex`.
#[cfg(target_os = "ios")]
unsafe impl<T> Sync for AnyThread<T> {}

#[cfg(target_os = "ios")]
impl AnyThread<Mutex<FetchGate>> {
    fn done(&self, result: usize) {
        let completion = {
            let mut gate = self.0.lock();
            if result == FETCH_NEW_DATA {
                gate.result = FETCH_NEW_DATA;
            }
            gate.remaining = gate.remaining.saturating_sub(1);
            if gate.remaining > 0 {
                return;
            }
            gate.completion
                .take()
                .map(|completion| (completion, gate.result))
        };
        if let Some((completion, result)) = completion {
            completion.call((result,));
        }
    }
}

#[cfg(target_os = "ios")]
unsafe extern "C-unwind" fn did_receive_remote_notification(
    this: &AnyObject,
    cmd: Sel,
    application: *mut AnyObject,
    info: *mut NSDictionary,
    completion: &DynBlock<dyn Fn(usize)>,
) {
    let original = original_with(this, cmd);
    let gate = Arc::new(AnyThread(Mutex::new(FetchGate {
        remaining: if original.is_some() { 2 } else { 1 },
        result: FETCH_NO_DATA,
        completion: Some(completion.copy()),
    })));
    if let Some(original) = original {
        let gate = Arc::clone(&gate);
        let forwarded = RcBlock::new(move |result: usize| gate.done(result));
        // SAFETY: the original class implements this method with this signature, and gets a block it may call from any thread.
        unsafe {
            let _: () = msg_send![super(this, original), application: application, didReceiveRemoteNotification: info, fetchCompletionHandler: &*forwarded];
        }
    }
    // SAFETY: the system passes an `NSDictionary` or null, valid for this call.
    let payload = unsafe { info.as_ref() }.and_then(|info| json(info));
    let message = payload.and_then(|payload| super::receive(payload, Arrival::Push));
    let Some(mtm) = MainThreadMarker::new() else {
        gate.done(FETCH_NO_DATA);
        return;
    };
    let result = if message.is_some() {
        FETCH_NEW_DATA
    } else {
        FETCH_NO_DATA
    };
    super::finish_after_background_work(mtm, message, Box::new(move || gate.done(result)));
}

#[cfg(target_os = "macos")]
unsafe extern "C-unwind" fn did_receive_remote_notification(
    this: &AnyObject,
    cmd: Sel,
    application: *mut AnyObject,
    info: *mut NSDictionary,
) {
    if let Some(original) = original_with(this, cmd) {
        // SAFETY: the original class implements this method with this signature.
        unsafe {
            let _: () = msg_send![super(this, original), application: application, didReceiveRemoteNotification: info];
        }
    }
    // SAFETY: the system passes an `NSDictionary` or null, valid for this call.
    let payload = unsafe { info.as_ref() }.and_then(|info| json(info));
    let message = payload.and_then(|payload| super::receive(payload, Arrival::Push));
    if let Some(mtm) = MainThreadMarker::new() {
        super::finish_after_background_work(mtm, message, Box::new(|| {}));
    }
}

/// A tap on a notification. Only a remote notification's tap is a push, so a local one passes straight to the original method.
unsafe extern "C-unwind" fn did_receive_response(
    this: &AnyObject,
    cmd: Sel,
    center: *mut AnyObject,
    response: *mut AnyObject,
    completion: &DynBlock<dyn Fn()>,
) {
    // SAFETY: the system passes a `UNNotificationResponse`, valid for this call.
    if let Some(response) = unsafe { response.as_ref() } {
        // SAFETY: `notification` is a getter of `UNNotificationResponse`.
        let notification: Retained<AnyObject> = unsafe { msg_send![response, notification] };
        if let Some(payload) = remote_payload(&notification) {
            let _ = super::receive(payload, Arrival::Tap);
        }
    }
    match original_with(this, cmd) {
        // SAFETY: the original class implements this method with this signature, and calls the completion handler itself.
        Some(original) => unsafe {
            let _: () = msg_send![super(this, original), userNotificationCenter: center, didReceiveNotificationResponse: response, withCompletionHandler: completion];
        },
        None => completion.call(()),
    }
}

/// The `userInfo` of a remote `UNNotification` as JSON, or `None` for a local one.
fn remote_payload(notification: &AnyObject) -> Option<Vec<u8>> {
    // SAFETY: each getter belongs to the class it is sent to, `UNNotification`, `UNNotificationRequest` and `UNNotificationContent`.
    unsafe {
        let request: Retained<AnyObject> = msg_send![notification, request];
        let trigger: Option<Retained<AnyObject>> = msg_send![&*request, trigger];
        let remote: Bool = match &trigger {
            Some(trigger) => {
                msg_send![&**trigger, isKindOfClass: class!(UNPushNotificationTrigger)]
            }
            None => Bool::NO,
        };
        if !remote.as_bool() {
            return None;
        }
        let content: Retained<AnyObject> = msg_send![&*request, content];
        let info: Retained<AnyObject> = msg_send![&*content, userInfo];
        json(&info)
    }
}

/// A push arriving while the app is in front. iOS hands an alert without `content-available` to no other method, so this delivers it, and the delivered set drops the copy a `content-available` push also brings through `didReceiveRemoteNotification`. The presentation stays the app's own delegate's choice, and the crate's own delegate shows it as the system shows a background push.
unsafe extern "C-unwind" fn will_present(
    this: &AnyObject,
    cmd: Sel,
    center: *mut AnyObject,
    notification: *mut AnyObject,
    completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
) {
    // SAFETY: the system passes a `UNNotification`, valid for this call.
    let payload = unsafe { notification.as_ref() }.and_then(remote_payload);
    let message = payload.and_then(|payload| super::receive(payload, Arrival::Push));
    if let Some(mtm) = MainThreadMarker::new() {
        super::finish_after_background_work(mtm, message, Box::new(|| {}));
    }
    if let Some(original) = original_with(this, cmd) {
        // SAFETY: the original class implements this method with this signature, and calls the completion handler itself.
        unsafe {
            let _: () = msg_send![super(this, original), userNotificationCenter: center, willPresentNotification: notification, withCompletionHandler: completion];
        }
    } else if this.class().name() == OWN_NOTIFICATION_DELEGATE {
        completion.call((UNNotificationPresentationOptions::Banner
            | UNNotificationPresentationOptions::List
            | UNNotificationPresentationOptions::Sound,));
    } else {
        // An app delegate that never chose a presentation keeps the system's default, nothing shown.
        completion.call((UNNotificationPresentationOptions::empty(),));
    }
}

/// The app set a new notification delegate, which the crate extends.
unsafe extern "C-unwind" fn delegate_changed(
    _this: &AnyObject,
    _cmd: Sel,
    _key_path: *mut NSString,
    _object: *mut AnyObject,
    _change: *mut NSDictionary,
    _context: *mut c_void,
) {
    let center = UNUserNotificationCenter::currentNotificationCenter();
    adopt_notification_delegate(&center);
}
