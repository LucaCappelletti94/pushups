package rs.pushups.ci

import android.os.ParcelFileDescriptor
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import org.junit.AfterClass
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.BeforeClass
import org.junit.FixMethodOrder
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.MethodSorters

/**
 * Opens the notification setting through `pushups::open_notification_settings`, checks the
 * setting the user lands in is the settings app's and returns home, then disables the settings
 * app and checks the call reports the refusal instead of opening anything.
 */
@RunWith(AndroidJUnit4::class)
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class NotificationSettingsTest {

    companion object {
        private const val SETTINGS = "com.android.settings"
        private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
        private val device: UiDevice get() = UiDevice.getInstance(instrumentation)

        @BeforeClass
        @JvmStatic
        fun install() {
            assertNull(Probe.install())
        }

        @AfterClass
        @JvmStatic
        fun finish() = writeCoverage()

        /** Runs [command] as the shell user and returns its output. */
        private fun shell(command: String): String =
            ParcelFileDescriptor.AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command))
                .bufferedReader()
                .use { it.readText() }
    }

    @Test
    fun a1_theNotificationSettingOpensTheSettingsApp() {
        assertNull(Probe.openNotificationSettings())
        val current = waitFor(UI_MS) {
            val name = device.currentPackageName
            if (name == SETTINGS) name else null
        }
        assertEquals(SETTINGS, current)
        device.pressHome()
    }

    @Test
    fun a2_aDeviceWithNoSettingAppReportsTheRefusal() {
        val disabled = shell("pm disable-user --user 0 $SETTINGS")
        check("disabled-user" in disabled) { "the shell could not disable $SETTINGS: $disabled" }
        try {
            val error = Probe.openNotificationSettings()
            assertTrue("$error", error?.startsWith("push service error: ") == true)
        } finally {
            shell("pm enable $SETTINGS")
        }
    }
}
