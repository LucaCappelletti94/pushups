package rs.pushups

import android.util.Log
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import org.json.JSONObject
import java.nio.charset.StandardCharsets

/**
 * Firebase's service for this app. Persists every push and token through the
 * Rust exports, then runs the app's background handler when one exists.
 */
class PushupsMessagingService : FirebaseMessagingService() {

    override fun onNewToken(token: String) {
        invokeNative("onToken") { Native.onToken(token) }
    }

    override fun onMessageReceived(message: RemoteMessage) {
        Log.d(TAG, "message ${message.messageId} received, priority ${message.priority}")
        if (!ProcessState.loaded) {
            Log.w(TAG, "message received before the native library loaded, dropping it")
            return
        }
        val payload = JSONObject(message.data).toString().toByteArray(StandardCharsets.UTF_8)
        val startedApp = ProcessState.pushStartedApp()
        invokeNative("onMessage") { Native.onMessage(payload, startedApp) }
        try {
            BackgroundHandler.handle(applicationContext, payload, startedApp)
        } catch (e: UnsatisfiedLinkError) {
            Log.i(TAG, "no background handler, the queue keeps the push")
        } catch (e: RuntimeException) {
            Log.e(TAG, "background handler", e)
        }
    }

    override fun onDeletedMessages() {
        invokeNative("onMessagesDropped") { Native.onMessagesDropped() }
    }
}
