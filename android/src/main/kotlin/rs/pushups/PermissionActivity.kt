package rs.pushups

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log

/**
 * Transparent, no-UI activity that only drives the POST_NOTIFICATIONS prompt.
 * Replies with the permission's real state exactly once, on the result, on
 * destroy, or after a 10-minute bound. A rebuild for a configuration change
 * keeps the request, since the system hands the prompt's result to the new
 * instance.
 */
class PermissionActivity : Activity() {

    private val handler = Handler(Looper.getMainLooper())
    private var requestId: Long = 0
    private var replied = false
    private var deadline: Long = 0

    private val timeout = Runnable {
        Log.w(TAG, "permission request $requestId timed out, replying with the current state")
        replyWith(currentState())
        finish()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        requestId = intent.getLongExtra(EXTRA_REQUEST_ID, 0)
        if (savedInstanceState == null) {
            deadline = SystemClock.elapsedRealtime() + TIMEOUT_MS
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), REQUEST_CODE)
        } else {
            deadline = savedInstanceState.getLong(KEY_DEADLINE)
        }
        handler.postDelayed(timeout, maxOf(0L, deadline - SystemClock.elapsedRealtime()))
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        outState.putLong(KEY_DEADLINE, deadline)
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode != REQUEST_CODE) return
        handler.removeCallbacks(timeout)
        val granted = grantResults.isNotEmpty() &&
            grantResults[0] == PackageManager.PERMISSION_GRANTED
        Pushups.markPromptAnswered(this)
        replyWith(granted)
        finish()
    }

    override fun onDestroy() {
        handler.removeCallbacks(timeout)
        if (!replied && !isChangingConfigurations) replyWith(currentState())
        super.onDestroy()
    }

    private fun currentState(): Boolean =
        checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED

    private fun replyWith(granted: Boolean) {
        if (replied) return
        replied = true
        invokeNative("onPermissionResult") { Native.onPermissionResult(requestId, granted) }
    }

    companion object {
        const val EXTRA_REQUEST_ID = "requestId"

        private const val REQUEST_CODE = 1001

        private const val KEY_DEADLINE = "rs.pushups.deadline"

        private const val TIMEOUT_MS = 10 * 60 * 1000L
    }
}
