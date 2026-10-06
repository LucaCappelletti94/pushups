package rs.pushups

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationManagerCompat
import com.google.firebase.messaging.FirebaseMessaging

/**
 * Entry points the Rust crate calls through JNI. Holds the application context
 * the provider set. Results are delivered back through the Native callbacks.
 */
object Pushups {

    @Volatile
    var context: Context? = null

    /** Asks Firebase for a token, which arrives through [Native.onRegistered]. */
    @JvmStatic
    fun register() {
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
