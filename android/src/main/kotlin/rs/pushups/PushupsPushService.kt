package rs.pushups

import android.util.Base64
import android.util.Log
import org.unifiedpush.android.connector.FailedReason
import org.unifiedpush.android.connector.PushService
import org.unifiedpush.android.connector.data.PushEndpoint
import org.unifiedpush.android.connector.data.PushMessage

/**
 * The UnifiedPush connector's service for this app. The connector decrypts each message before
 * calling here, and every event goes to the same Rust exports as FCM's.
 */
class PushupsPushService : PushService() {

    override fun onNewEndpoint(endpoint: PushEndpoint, instance: String) {
        // Rust turns keys that are missing or of the wrong length into a persisted failure.
        val keys = endpoint.pubKeySet
        val p256dh = keys?.let { base64url(it.pubKey) } ?: ByteArray(0)
        val auth = keys?.let { base64url(it.auth) } ?: ByteArray(0)
        invokeNative("onWebPushToken") { Native.onWebPushToken(endpoint.url, p256dh, auth) }
    }

    override fun onMessage(message: PushMessage, instance: String) {
        if (!message.decrypted) {
            Log.w(TAG, "UnifiedPush message the connector could not decrypt, dropping it")
            return
        }
        deliverPush(applicationContext, message.content)
    }

    override fun onRegistrationFailed(reason: FailedReason, instance: String) {
        invokeNative("onRegistered") {
            Native.onRegistered(null, "the distributor refused the registration: $reason")
        }
    }

    override fun onUnregistered(instance: String) {
        invokeNative("onUnregistered") { Native.onUnregistered() }
    }

    private fun base64url(text: String): ByteArray =
        Base64.decode(text, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)
}
