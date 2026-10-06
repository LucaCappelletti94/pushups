package rs.pushups

import android.app.Activity
import android.app.Application
import android.content.Context
import android.content.SharedPreferences
import android.os.Bundle
import org.json.JSONArray
import org.json.JSONObject
import java.nio.charset.StandardCharsets

/**
 * Tracks the live UI session, the pushes the process has seen, and the last
 * delivered tap ids. Runs as the app's [Application.ActivityLifecycleCallbacks].
 */
object ProcessState : Application.ActivityLifecycleCallbacks {
    private const val PREFS = "pushups"
    private const val KEY_IDS = "delivered_message_ids"
    private const val MAX_IDS = 64
    private const val MSG_ID = "google.message_id"
    private val DROPPED = setOf("from", "message_type", "collapse_key")

    private val lock = Any()
    private val delivered = ArrayDeque<String>()
    @Volatile
    private var idsLoaded = false

    @Volatile
    var loaded: Boolean = false

    @Volatile
    private var sessionActive = false

    @Volatile
    private var liveActivities = 0

    @Volatile
    private var everCreatedActivity = false

    @Volatile
    private var pushesSeen = 0

    override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) {
        val (firstActivity, newSession) = synchronized(lock) {
            val first = !everCreatedActivity
            everCreatedActivity = true
            val started = !sessionActive
            sessionActive = true
            liveActivities++
            first to started
        }
        if (newSession) invokeNative("onSession") { Native.onSession(true) }
        handleTap(activity, firstActivity)
    }

    override fun onActivityDestroyed(activity: Activity) {
        val emptied = synchronized(lock) {
            liveActivities--
            liveActivities == 0
        }
        if (emptied && !activity.isChangingConfigurations) {
            sessionActive = false
            invokeNative("onSession") { Native.onSession(false) }
        }
    }

    override fun onActivityStarted(activity: Activity) {}

    override fun onActivityResumed(activity: Activity) {}

    override fun onActivityPaused(activity: Activity) {}

    override fun onActivityStopped(activity: Activity) {}

    override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) {}

    /**
     * Records this push and reports whether the app was started for it: the first push of a
     * process that never created an Activity.
     */
    fun pushStartedApp(): Boolean = synchronized(lock) {
        val startedApp = pushesSeen == 0 && !everCreatedActivity
        pushesSeen++
        startedApp
    }

    /** Whether a tap started the app: its Activity is the first of a process that handled no push. */
    private fun tapStartedApp(firstActivity: Boolean): Boolean =
        firstActivity && synchronized(lock) { pushesSeen == 0 }

    private fun handleTap(activity: Activity, firstActivity: Boolean) {
        val id = activity.intent?.getStringExtra(MSG_ID) ?: return
        val prefs = activity.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        loadDelivered(prefs)
        if (synchronized(lock) { delivered.contains(id) }) return
        val startedApp = tapStartedApp(firstActivity)
        val bytes = tapPayload(activity).toString().toByteArray(StandardCharsets.UTF_8)
        invokeNative("onMessage") { Native.onMessage(bytes, startedApp) }
        rememberDelivered(prefs, id)
    }

    private fun tapPayload(activity: Activity): JSONObject {
        val json = JSONObject()
        val extras = activity.intent?.extras ?: return json
        for (key in extras.keySet()) {
            if (key.startsWith("google.") || key.startsWith("gcm.")) continue
            if (key in DROPPED) continue
            val value = extras.getString(key)
            if (value != null) json.put(key, value)
        }
        return json
    }

    private fun loadDelivered(prefs: SharedPreferences) {
        if (idsLoaded) return
        synchronized(lock) {
            if (idsLoaded) return
            try {
                val arr = JSONArray(prefs.getString(KEY_IDS, "[]"))
                for (i in 0 until arr.length()) delivered.addLast(arr.getString(i))
            } catch (e: Exception) {
                // A corrupt or empty list starts the history afresh.
            }
            idsLoaded = true
        }
    }

    private fun rememberDelivered(prefs: SharedPreferences, id: String) {
        synchronized(lock) {
            delivered.addLast(id)
            while (delivered.size > MAX_IDS) delivered.removeFirst()
            val arr = JSONArray()
            delivered.forEach { arr.put(it) }
            // Written before returning, so a process that dies right after the tap still
            // knows the id when its Activity is restored with the same intent.
            prefs.edit().putString(KEY_IDS, arr.toString()).commit()
        }
    }
}
