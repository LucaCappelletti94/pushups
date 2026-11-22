package rs.pushups.ci

import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.os.Build
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.AfterClass
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.BeforeClass
import org.junit.Test
import org.junit.runner.RunWith

/**
 * An app whose manifest names an icon uses it as the notification's small icon, which the
 * module's presentation would otherwise fall back to the system's default for.
 */
@RunWith(AndroidJUnit4::class)
class IconTest {

    companion object {
        private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
        private val context: Context get() = instrumentation.targetContext
        private val notifications: NotificationManager
            get() = context.getSystemService(NotificationManager::class.java)

        @BeforeClass
        @JvmStatic
        fun setUp() {
            if (Build.VERSION.SDK_INT >= 33) {
                instrumentation.uiAutomation.grantRuntimePermission(
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
    fun theAppIconIsUsedAsTheSmallIcon() {
        val tag = "icon-${java.util.UUID.randomUUID()}"
        assertNull(Probe.present("pushups icon", "shown", null, tag, null, null))
        if (Build.VERSION.SDK_INT >= 26) {
            val resId = context.resources.getIdentifier("pushups_icon", "drawable", context.packageName)
            val notification = waitFor(UI_MS) {
                notifications.activeNotifications.firstOrNull { it.tag == tag }?.notification
            } ?: error("the notification $tag did not settle within ${UI_MS / 1000} s")
            val icon = notification.getSmallIcon()
            assertEquals(context.packageName, icon.getResPackage())
            assertEquals(resId, icon.getResId())
        }
    }
}
