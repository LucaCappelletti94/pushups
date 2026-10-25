package rs.pushups

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
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

    /**
     * Reports the state of the notification permission without asking, as one of
     * [STATUS_GRANTED], [STATUS_DENIED] and [STATUS_NOT_DETERMINED].
     */
    @JvmStatic
    fun permissionStatus(): Int {
        val appContext = context ?: return STATUS_NOT_DETERMINED
        if (Build.VERSION.SDK_INT < 33) {
            return if (enabled(appContext)) STATUS_GRANTED else STATUS_DENIED
        }
        val granted = appContext.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
        return when {
            granted -> if (enabled(appContext)) STATUS_GRANTED else STATUS_DENIED
            asked(appContext) -> STATUS_DENIED
            else -> STATUS_NOT_DETERMINED
        }
    }

    /** Presents through the application context and returns `null` on success or an error message. */
    @JvmStatic
    fun present(
        title: String,
        body: String?,
        navigate: String?,
        tag: String?,
        icon: String?,
        data: String?,
    ): String? {
        val appContext = context ?: return "the module has no application context"
        return Presentation.present(appContext, title, body, navigate, tag, icon, data)
    }

    /** Reports the transport this app's pushes come through, which Rust's key rotation reads. */
    @JvmStatic
    fun transport(): String = context?.let { Transport.get(it) } ?: Transport.FCM

    private fun enabled(context: Context): Boolean =
        NotificationManagerCompat.from(context).areNotificationsEnabled()

    private fun asked(context: Context): Boolean =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean(KEY_ASKED, false)

    /**
     * Remembers that the prompt answered, so [permissionStatus] can tell a refusal from
     * an app the user has never been asked.
     */
    internal fun markPromptAnswered(context: Context) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .edit().putBoolean(KEY_ASKED, true).apply()
    }

    private const val PREFS = "pushups"
    private const val KEY_ASKED = "permission_prompt_answered"

    /** [permissionStatus]'s answer, the permission granted and notifications enabled. */
    const val STATUS_GRANTED = 0

    /** [permissionStatus]'s answer, the user or the system refusing notifications. */
    const val STATUS_DENIED = 1

    /** [permissionStatus]'s answer, the user has not been asked yet. */
    const val STATUS_NOT_DETERMINED = 2
}
