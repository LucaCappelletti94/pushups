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
        // Firebase makes a token at start on its own, which an app on UnifiedPush does not use.
        val context = Pushups.context ?: return
        if (Transport.get(context) == Transport.UNIFIED_PUSH) {
            Log.i(TAG, "FCM token dropped, the app's pushes come through UnifiedPush")
            return
        }
        invokeNative("onToken") { Native.onToken(token) }
    }

    override fun onMessageReceived(message: RemoteMessage) {
        Log.d(TAG, "message ${message.messageId} received, priority ${message.priority}")
        deliverPush(applicationContext, JSONObject(message.data).toString().toByteArray(StandardCharsets.UTF_8))
    }

    override fun onDeletedMessages() {
        invokeNative("onMessagesDropped") { Native.onMessagesDropped() }
    }
}
