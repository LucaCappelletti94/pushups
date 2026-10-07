package rs.pushups

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationManagerCompat
import com.google.firebase.messaging.FirebaseMessaging
import org.unifiedpush.android.connector.UnifiedPush

/**
 * Entry points the Rust crate calls through JNI. Holds the application context
 * the provider set. Results are delivered back through the Native callbacks.
 */
object Pushups {

    @Volatile
    var context: Context? = null

    /**
     * Asks for a token, which arrives through [Native.onRegistered] for FCM or
     * [Native.onWebPushToken] for UnifiedPush. With a [vapid] key a distributor carries the
     * pushes when one is installed, and FCM otherwise.
     */
    @JvmStatic
    fun register(vapid: String?) {
        val appContext = context ?: return
        if (vapid != null) {
            val saved = UnifiedPush.getAckDistributor(appContext) ?: UnifiedPush.getSavedDistributor(appContext)
            if (saved != null) {
                registerUnifiedPush(appContext, vapid)
                return
            }
            if (UnifiedPush.getDistributors(appContext).isNotEmpty()) {
                // The connector's picker needs an Activity, which the toolkit's own may not be.
                val intent = Intent(appContext, DistributorActivity::class.java)
                intent.putExtra(DistributorActivity.EXTRA_VAPID, vapid)
                intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                appContext.startActivity(intent)
                return
            }
        }
        registerFcm(appContext)
    }

    /**
     * Registers with the saved distributor, whose endpoint arrives in [PushupsPushService]. The
     * [vapid] key is the base64url of the 65 bytes Rust checked, so the connector accepts it.
     */
    internal fun registerUnifiedPush(appContext: Context, vapid: String) {
        Transport.set(appContext, Transport.UNIFIED_PUSH)
        UnifiedPush.register(appContext, "default", null, vapid)
    }

    /** Asks Firebase for a token, which arrives through [Native.onRegistered]. */
    internal fun registerFcm(appContext: Context) {
        Transport.set(appContext, Transport.FCM)
        try {
            FirebaseMessaging.getInstance().token.addOnCompleteListener { task ->
                if (task.isSuccessful) {
                    invokeNative("onRegistered") { Native.onRegistered(task.result, null) }
                } else {
                    invokeNative("onRegistered") {
                        Native.onRegistered(null, task.exception?.message ?: "unknown")
                    }
                }
            }
        } catch (e: IllegalStateException) {
            invokeNative("onRegistered") {
                Native.onRegistered(null, "FCM is not configured: ${e.message}")
            }
        }
    }

    /**
     * Requests `POST_NOTIFICATIONS`. The answer arrives through [Native.onPermissionResult]
     * with the same [requestId].
     */
    @JvmStatic
    fun requestPermission(requestId: Long) {
        val appContext = context ?: return
        if (Build.VERSION.SDK_INT < 33) {
            invokeNative("onPermissionResult") {
                Native.onPermissionResult(
                    requestId,
                    NotificationManagerCompat.from(appContext).areNotificationsEnabled()
                )
            }
            return
        }
        if (appContext.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
        ) {
            invokeNative("onPermissionResult") { Native.onPermissionResult(requestId, true) }
            return
        }
        val intent = Intent(appContext, PermissionActivity::class.java)
        intent.putExtra(PermissionActivity.EXTRA_REQUEST_ID, requestId)
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        appContext.startActivity(intent)
    }
}
