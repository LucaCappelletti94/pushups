package rs.pushups.ci

import android.Manifest
import android.content.Context
import android.os.Build
import android.os.ParcelFileDescriptor
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import org.junit.AfterClass
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assume.assumeTrue
import org.junit.BeforeClass
import org.junit.FixMethodOrder
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.MethodSorters
import java.util.regex.Pattern

/**
 * The module's prompt Activity with the edge results the system can hand it. An answer for
 * another request code is ignored, and an Activity destroyed before the system answers replies
 * with the permission's real state. The steps run in a fresh process, so each request starts
 * ungranted.
 */
@RunWith(AndroidJUnit4::class)
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class PromptResultTest {

    companion object {
        private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
        private val context: Context get() = instrumentation.targetContext
        private val device: UiDevice get() = UiDevice.getInstance(instrumentation)

        @BeforeClass
        @JvmStatic
        fun setUp() {
            assertNull(Probe.install())
            Probe.setHandler()
        }

        @AfterClass
        @JvmStatic
        fun finish() = writeCoverage()

        private fun awaitPermission(): String =
            waitFor(UI_MS) { Probe.permission() } ?: error("no permission answer within ${UI_MS / 1000} s")
    }

    @Test
    fun a01_aResultForAnotherRequestCodeIsIgnored() {
        assumeTrue(Build.VERSION.SDK_INT >= 33)
        val monitor = instrumentation.addMonitor("rs.pushups.PermissionActivity", null, false)
        try {
            Probe.startPermission()
            val prompt = waitFor(UI_MS) { monitor.lastActivity }
                ?: error("no PermissionActivity within ${UI_MS / 1000} s")
            // A result for another request code is ignored, and the request stays pending.
            prompt.onRequestPermissionsResult(
                4242, arrayOf(Manifest.permission.POST_NOTIFICATIONS), intArrayOf(0)
            )
            assertNull(Probe.permission())
            // The system's own result still answers the request, and finishes the Activity.
            assertEquals("Denied", answerPrompt("permission_deny_button"))
            waitFor(UI_MS) { if (prompt.isFinishing) true else null }
                ?: error("the prompt Activity did not finish within ${UI_MS / 1000} s")
        } finally {
            instrumentation.removeMonitor(monitor)
        }
    }

    @Test
    fun a02_aDestroyedPromptActivityRepliesWithTheCurrentState() {
        assumeTrue(Build.VERSION.SDK_INT >= 33)
        val monitor = instrumentation.addMonitor("rs.pushups.PermissionActivity", null, false)
        try {
            Probe.startPermission()
            val prompt = waitFor(UI_MS) { monitor.lastActivity }
                ?: error("no PermissionActivity within ${UI_MS / 1000} s")
            // Destroyed before any answer, the Activity replies with the permission's real state.
            prompt.finish()
            instrumentation.waitForIdleSync()
            assertEquals("Denied", awaitPermission())
        } finally {
            instrumentation.removeMonitor(monitor)
            // The request's system prompt may still be up; dismiss it.
            runCatching {
                device.findObject(By.res("com.android.permissioncontroller", "permission_deny_button"))?.click()
            }
        }
    }

    @Test
    fun a03_aPromptActivityRecreatedByAConfigurationChangeKeepsTheRequest() {
        assumeTrue(Build.VERSION.SDK_INT >= 33)
        // The step above may leave its system prompt up, in the task a new request would join.
        waitFor(UI_MS) {
            if (device.findObject(By.pkg("com.google.android.permissioncontroller")) == null) {
                true
            } else {
                // A second request's prompt names its refusal permission_deny_and_dont_ask_again_button.
                runCatching {
                    device.findObject(By.res(Pattern.compile("com.android.permissioncontroller:id/permission_deny.*")))?.click()
                }
                null
            }
        } ?: error("the earlier system prompt stayed up for ${UI_MS / 1000} s")
        // The steps above denied the permission twice, after which the system stops asking.
        val permission = Manifest.permission.POST_NOTIFICATIONS
        ParcelFileDescriptor.AutoCloseInputStream(
            instrumentation.uiAutomation.executeShellCommand(
                "pm clear-permission-flags ${context.packageName} $permission user-set user-fixed"
            )
        ).use { it.readBytes() }
        val monitor = instrumentation.addMonitor("rs.pushups.PermissionActivity", null, false)
        try {
            Probe.startPermission()
            val prompt = waitFor(UI_MS) { monitor.lastActivity }
                ?: error("no PermissionActivity within ${UI_MS / 1000} s")
            // A rotation or a theme change rebuilds the Activity while the system prompt is up.
            instrumentation.runOnMainSync { prompt.recreate() }
            val rebuilt = waitFor(UI_MS) { monitor.lastActivity?.takeIf { it !== prompt } }
                ?: error("the PermissionActivity was not rebuilt within ${UI_MS / 1000} s")
            assertNull(Probe.permission())
            // The user's answer reaches the rebuilt Activity, which replies with it.
            assertEquals("Granted", answerPrompt("permission_allow_button"))
            waitFor(UI_MS) { if (rebuilt.isFinishing) true else null }
                ?: error("the rebuilt prompt Activity did not finish within ${UI_MS / 1000} s")
        } finally {
            instrumentation.removeMonitor(monitor)
        }
    }
}
