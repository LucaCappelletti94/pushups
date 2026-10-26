package rs.pushups.ci

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import org.junit.AfterClass
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.BeforeClass
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Opens the notification setting through `pushups::open_notification_settings`, checks the
 * setting the user lands in is the settings app's, and returns home.
 */
@RunWith(AndroidJUnit4::class)
class NotificationSettingsTest {

    companion object {
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
    }

    @Test
    fun theNotificationSettingOpensTheSettingsApp() {
        assertNull(Probe.openNotificationSettings())
        val current = waitFor(UI_MS) {
            val name = device.currentPackageName
            if (name == "com.android.settings") name else null
        }
        assertEquals("com.android.settings", current)
        device.pressHome()
    }
}
