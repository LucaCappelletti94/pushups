package rs.pushups

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import java.nio.charset.StandardCharsets
import java.util.UUID

/** Presents through the application context and preserves tap data without opening an `Activity`. */
internal object Presentation {

    /** The module's channel, at default importance, labelled with the application's name. */
    const val CHANNEL_ID = "pushups"

    /** The tap's id extra, which the delivered-ids history deduplicates. */
    const val TAP_ID = "rs.pushups.tap"

    /** The tap's data extra, the exact bytes a tap delivers as the message's payload. */
    const val TAP_DATA = "rs.pushups.data"

    /** The tap's navigation extra, which a toolkit may open on a tap. */
    const val NAVIGATE = "rs.pushups.navigate"

    /** Returns `null` on success or an error message on refusal. */
    fun present(
        context: Context,
        title: String,
        body: String?,
        navigate: String?,
        tag: String?,
        icon: String?,
        data: String?,
    ): String? {
        if (icon != null) return "an icon URL is not supported on Android"
        if (Build.VERSION.SDK_INT >= 33 &&
            context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            return "notifications are not allowed"
        }
        val manager = NotificationManagerCompat.from(context)
        if (!manager.areNotificationsEnabled()) return "notifications are disabled"
        if (Build.VERSION.SDK_INT >= 26) {
            if (manager.getNotificationChannel(CHANNEL_ID) == null) {
                manager.createNotificationChannel(
                    NotificationChannel(
                        CHANNEL_ID,
                        context.applicationInfo.loadLabel(context.packageManager).toString(),
                        NotificationManager.IMPORTANCE_DEFAULT,
                    )
                )
            }
            if (manager.getNotificationChannel(CHANNEL_ID)?.importance == NotificationManager.IMPORTANCE_NONE) {
                return "the pushups channel is disabled"
            }
        }
        val launcher = context.packageManager.getLaunchIntentForPackage(context.packageName)
            ?: return "the app has no launcher activity"
        val identity = tag ?: UUID.randomUUID().toString()
        val launch = launcher.apply {
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            addCategory("rs.pushups.notification.$identity")
            putExtra(TAP_ID, UUID.randomUUID().toString())
            if (navigate != null) {
                setData(Uri.parse(navigate))
                putExtra(NAVIGATE, navigate)
            }
            if (data != null) putExtra(TAP_DATA, data.toByteArray(StandardCharsets.UTF_8))
        }
        val tap = PendingIntent.getActivity(
            context,
            0,
            launch,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val builder = NotificationCompat.Builder(context, CHANNEL_ID).apply {
            setContentTitle(title)
            if (body != null) setContentText(body)
            val iconResource = context.applicationInfo.icon
            setSmallIcon(if (iconResource == 0) android.R.drawable.sym_def_app_icon else iconResource)
            setAutoCancel(true)
            setContentIntent(tap)
        }
        manager.notify(identity, 0, builder.build())
        return null
    }
}
