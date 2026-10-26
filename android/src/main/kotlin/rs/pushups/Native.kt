package rs.pushups

import android.content.Context
import android.util.Log

/** Rust-implemented exports in the app's cdylib, symbols Java_rs_pushups_Native_*. */
object Native {
    /**
     * Hands Rust this module's version and returns whether it equals the crate's, which every
     * other export depends on. Its signature never changes, so any two versions can make this call.
     */
    @JvmStatic
    external fun handshake(moduleVersion: String): Boolean

    @JvmStatic
    external fun init(context: Context, filesDir: String)

    @JvmStatic
    external fun onSession(active: Boolean)

    @JvmStatic
    external fun onToken(token: String)

    @JvmStatic
    external fun onMessage(payload: ByteArray, startedApp: Boolean)

    @JvmStatic
    external fun onMessagesDropped()

    @JvmStatic
    external fun onRegistered(token: String?, error: String?)

    @JvmStatic
    external fun onWebPushToken(endpoint: String, p256dh: ByteArray, auth: ByteArray)

    @JvmStatic
    external fun onUnregistered()

    @JvmStatic
    external fun onPermissionResult(requestId: Long, granted: Boolean)

    /** Whether the crate's `encryption` feature was on, which the module asks once. */
    @JvmStatic
    external fun encrypts(): Boolean

    /** The plaintext of a sealed `pushups` field, or a thrown `RuntimeException` on failure. */
    @JvmStatic
    external fun openEnvelope(sealed: String): ByteArray
}

/** Log tag shared by every pushups line. */
const val TAG = "pushups"

/**
 * Calls a Rust export only once the provider loaded the cdylib, returns the export's value,
 * and logs the RuntimeException the Rust side throws instead of taking the app down.
 */
internal fun <T : Any> invokeNative(op: String, block: () -> T): T? {
    if (!ProcessState.loaded) {
        Log.w(TAG, "$op skipped, the native library is not loaded")
        return null
    }
    return try {
        block()
    } catch (e: RuntimeException) {
        Log.e(TAG, op, e)
        null
    }
}
