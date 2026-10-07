package rs.pushups.ci

/** The probe library's exports, which call `pushups` from Rust. PushupsProvider loads the library. */
object Probe {
    /** `null`, or the error of `pushups::install`. */
    @JvmStatic
    external fun install(): String?

    /** Sets a handler that records every event as a line. */
    @JvmStatic
    external fun setHandler()

    /** `null`, or the error of `pushups::register`. */
    @JvmStatic
    external fun register(): String?

    /** Starts `pushups::request_permission` on a thread of its own. */
    @JvmStatic
    external fun startPermission()

    /** The answer to [startPermission], `Granted`, `Denied` or `error ...`, or `null` while pending. */
    @JvmStatic
    external fun permission(): String?

    /** Every event the handler received, one per line. */
    @JvmStatic
    external fun events(): String

    /** Every push the background handler received, one per line. */
    @JvmStatic
    external fun handled(): String

    /** Writes the Rust coverage profile to [path]. */
    @JvmStatic
    external fun writeCoverage(path: String): Boolean
}
