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
}

/** Log tag shared by every pushups line. */
const val TAG = "pushups"

/**
 * Calls a Rust export only once the provider loaded the cdylib and logs the
 * RuntimeException the Rust side throws instead of taking the app down.
 */
internal fun invokeNative(op: String, block: () -> Unit) {
    if (!ProcessState.loaded) {
        Log.w(TAG, "$op skipped, the native library is not loaded")
        return
    }
    try {
        block()
    } catch (e: RuntimeException) {
        Log.e(TAG, op, e)
    }
}
