package rs.pushups.ci

/** The probe library's exports, which call `pushups` from Rust. PushupsProvider loads the library. */
object Probe {
    /** `null`, or the error of `pushups::install`. */
    @JvmStatic
    external fun install(): String?

    /** `null`, or the error of `pushups::install` with a UnifiedPush part for the base64url VAPID key. */
    @JvmStatic
    external fun installWithVapid(vapid: String): String?

    /** Sets a handler that records every event as a line. */
    @JvmStatic
    external fun setHandler()

    /** Installs a handler that panics on the selected event. */
    @JvmStatic
    external fun setPanickingHandler(kind: String, requestId: String)

    /** `null`, or the error of `pushups::register`. */
    @JvmStatic
    external fun register(): String?

    /** `null`, or the error of `pushups::rotate_keys`. */
    @JvmStatic
    external fun rotateKeys(): String?

    /** Starts `pushups::request_permission` on a thread of its own. */
    @JvmStatic
    external fun startPermission()

    /** The answer to [startPermission], `Granted`, `Denied` or `error ...`, or `null` while pending. */
    @JvmStatic
    external fun permission(): String?

    /** The answer of `pushups::permission_status`, `Granted`, `Denied`, `NotDetermined` or `error ...`. */
    @JvmStatic
    external fun permissionStatus(): String

    /** `null`, or the error of `pushups::open_notification_settings`. */
    @JvmStatic
    external fun openNotificationSettings(): String?

    /** `null`, or the error of `Notification::show` on the notification built from the fields. */
    @JvmStatic
    fun present(title: String, body: String?, navigate: String?, tag: String?, icon: String?, data: String?): String? =
        presentBytes(title, body, navigate, tag, icon, data?.toByteArray(Charsets.UTF_8))

    /** [present] with `data` as UTF-8 bytes, which JNI string decoding cannot corrupt. */
    @JvmStatic
    external fun presentBytes(title: String, body: String?, navigate: String?, tag: String?, icon: String?, data: ByteArray?): String?

    /** Every event the handler received, one per line. */
    @JvmStatic
    external fun events(): String

    /** Every push the background handler received, one per line. */
    @JvmStatic
    external fun handled(): String

    /** Every push the rendering handler received, one per line. */
    @JvmStatic
    external fun rendered(): String

    /** Writes the Rust coverage profile to [path]. */
    @JvmStatic
    external fun writeCoverage(path: String): Boolean

    /** `true`, the generated export's success boolean. */
    @JvmStatic
    external fun boundaryTrue(): Boolean

    /** `null`, the generated export's null string return. */
    @JvmStatic
    external fun boundaryNull(): String?

    /** A fresh Java string echoing [id], the generated export's object return. */
    @JvmStatic
    external fun boundaryObject(id: String): String

    /** Fails a JNI call inside the export; the guard must throw a `RuntimeException`. */
    @JvmStatic
    external fun boundaryError(): String?

    /** Panics inside the export; the guard must catch it and throw a `RuntimeException`. */
    @JvmStatic
    external fun boundaryPanic(id: String)

    /** Records `boundary nested <id>` when its body runs, which the guard must not do while [boundaryPending] leaves an exception pending. */
    @JvmStatic
    external fun boundaryNested(id: String): String?

    /** Leaves a distinctive `IllegalStateException` pending, then enters [boundaryNested]'s entrypoint through it. */
    @JvmStatic
    external fun boundaryPending(id: String)

    /** Leaves a distinctive `IllegalStateException` pending and then panics. */
    @JvmStatic
    external fun boundaryPanicPending(id: String)

    /** Panics with a payload whose destructor panics. */
    @JvmStatic
    external fun boundaryPanickingDrop(id: String)

    /** Returns `recovered <id>`, proving the process survives the guard's failures. */
    @JvmStatic
    external fun boundaryRecovery(id: String): String
}
