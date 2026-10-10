package rs.pushups

import android.content.Context
import android.util.Log

/**
 * Hands one push to Rust, which persists or emits it, then runs the app's background handler
 * when one exists. FCM and UnifiedPush both deliver through here.
 */
internal fun deliverPush(context: Context, payload: ByteArray) {
    if (!ProcessState.loaded) {
        Log.w(TAG, "push received before the native library loaded, dropping it")
        return
    }
    val startedApp = ProcessState.pushStartedApp()
    invokeNative("onMessage") { Native.onMessage(payload, startedApp) }
    try {
        BackgroundHandler.handle(context, payload, startedApp)
    } catch (e: UnsatisfiedLinkError) {
        Log.i(TAG, "no background handler, the queue keeps the push")
    } catch (e: RuntimeException) {
        Log.e(TAG, "background handler", e)
    }
}

/** Which push service carries the app's pushes, kept so a process started for a push knows it. */
internal object Transport {
    private const val PREFS = "pushups"
    private const val KEY = "transport"
    const val FCM = "fcm"
    const val UNIFIED_PUSH = "unifiedpush"

    fun get(context: Context): String =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(KEY, null) ?: FCM

    fun set(context: Context, transport: String) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString(KEY, transport).apply()
    }
}
