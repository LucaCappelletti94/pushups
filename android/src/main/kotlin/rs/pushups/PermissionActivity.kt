package rs.pushups

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log

/**
 * Transparent, no-UI activity that only drives the POST_NOTIFICATIONS prompt.
 * Replies with the permission's real state exactly once, on the result, on
 * destroy, or after a 10-minute bound.
 */
class PermissionActivity : Activity() {

    private val handler = Handler(Looper.getMainLooper())
    private var requestId: Long = 0
    private var replied = false

    private val timeout = Runnable {
        Log.w(TAG, "permission request $requestId timed out, replying with the current state")
        replyWith(currentState())
        finish()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        requestId = intent.getLongExtra(EXTRA_REQUEST_ID, 0)
        requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), REQUEST_CODE)
        handler.postDelayed(timeout, TIMEOUT_MS)
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
        replyWith(granted)
        finish()
    }

    override fun onDestroy() {
        handler.removeCallbacks(timeout)
        if (!replied) replyWith(currentState())
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

        private const val TIMEOUT_MS = 10 * 60 * 1000L
    }
}
