package rs.pushups.ci

import android.content.Context
import android.os.Build
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.AfterClass
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.BeforeClass
import org.junit.FixMethodOrder
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.MethodSorters

/**
 * A manifest that lacks the module's permission Activity: the request reports the missing
 * Activity as its own error, and the thread still serves a later module call.
 */
@RunWith(AndroidJUnit4::class)
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class PermissionActivityMissingTest {

    companion object {
        private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
        private val context: Context get() = instrumentation.targetContext

        @BeforeClass
        @JvmStatic
        fun setUp() {
            if (Build.VERSION.SDK_INT >= 33) {
                instrumentation.uiAutomation.revokeRuntimePermission(
                    context.packageName,
                    android.Manifest.permission.POST_NOTIFICATIONS,
                )
            }
            assertNull(Probe.install())
            Probe.setHandler()
        }

        @AfterClass
        @JvmStatic
        fun finish() = writeCoverage()
    }

    @Test
    fun a01_theRequestReportsTheMissingActivity() {
        // Below 33 the request answers straight from the setting and no Activity is opened.
        assumeTrue(Build.VERSION.SDK_INT >= 33)
        val answer = Probe.requestPermissionNow()
        assertTrue(
            "the request $answer names the missing Activity",
            answer.startsWith("error ") && answer.contains("ActivityNotFoundException"),
        )
    }

    @Test
    fun a02_aLaterCallOnTheSameThreadStillSucceeds() {
        assumeTrue(Build.VERSION.SDK_INT >= 33)
        val status = Probe.permissionStatus()
        assertTrue(
            "the status $status is one the module reports, not the sticky exception",
            status == "Granted" || status == "Denied" || status == "NotDetermined",
        )
    }
}
