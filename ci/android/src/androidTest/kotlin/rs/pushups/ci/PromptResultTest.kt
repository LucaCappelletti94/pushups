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
 * another request code is ignored, an Activity destroyed before the system answers replies
 * with the permission's real state, a request while an earlier prompt lingers gets a prompt of
 * its own, concurrent requests share one prompt, and a rebuilt Activity keeps its request. The
 * steps run in a fresh process, so each request starts ungranted.
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

        /** Either refusal of the system prompt, which names it differently from the second request on. */
        private val denyButton =
            By.res(Pattern.compile("com.android.permissioncontroller:id/permission_deny.*"))

        /**
         * Dismisses a system prompt an earlier step left up, and clears the refusals the steps
         * gave, after two of which the system stops asking.
         */
        private fun freshPrompt() {
            waitFor(UI_MS) {
                if (device.findObject(By.pkg("com.google.android.permissioncontroller")) == null) {
                    true
                } else {
                    runCatching { device.findObject(denyButton)?.click() }
                    null
                }
            } ?: error("the earlier system prompt stayed up for ${UI_MS / 1000} s")
            ParcelFileDescriptor.AutoCloseInputStream(
                instrumentation.uiAutomation.executeShellCommand(
                    "pm clear-permission-flags ${context.packageName} " +
                        "${Manifest.permission.POST_NOTIFICATIONS} user-set user-fixed"
                )
            ).use { it.readBytes() }
        }
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
    fun a03_aRequestWhileAnEarlierPromptLingersGetsAPromptOfItsOwn() {
        assumeTrue(Build.VERSION.SDK_INT >= 33)
        freshPrompt()
        val monitor = instrumentation.addMonitor("rs.pushups.PermissionActivity", null, false)
        try {
            Probe.startPermission()
            val first = waitFor(UI_MS) { monitor.lastActivity }
                ?: error("no PermissionActivity within ${UI_MS / 1000} s")
            // Finished before any answer, the Activity leaves its system prompt up in its task.
            first.finish()
            instrumentation.waitForIdleSync()
            assertEquals("Denied", awaitPermission())
            Probe.startPermission()
            waitFor(UI_MS) { monitor.lastActivity?.takeIf { it !== first } }
                ?: error("the second request started no PermissionActivity within ${UI_MS / 1000} s")
            assertEquals("Denied", clickUntil(denyButton, UI_MS) { Probe.permission() })
        } finally {
            instrumentation.removeMonitor(monitor)
        }
    }

    @Test
    fun a04_requestsWhileAPromptIsUpShareItsAnswer() {
        assumeTrue(Build.VERSION.SDK_INT >= 33)
        freshPrompt()
        val monitor = instrumentation.addMonitor("rs.pushups.PermissionActivity", null, false)
        try {
            Probe.startPermission()
            waitFor(UI_MS) { device.findObject(denyButton) }
                ?: error("no system prompt within ${UI_MS / 1000} s")
            Probe.startPermission()
            // Both requests wait on the one prompt up, and its answer reaches both.
            assertEquals("Denied", clickUntil(denyButton, UI_MS) { Probe.permission() })
            assertEquals("Denied", awaitPermission())
            assertEquals(1, monitor.hits)
        } finally {
            instrumentation.removeMonitor(monitor)
        }
    }

    @Test
    fun a05_aPromptActivityRecreatedByAConfigurationChangeKeepsTheRequest() {
        assumeTrue(Build.VERSION.SDK_INT >= 33)
        freshPrompt()
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
