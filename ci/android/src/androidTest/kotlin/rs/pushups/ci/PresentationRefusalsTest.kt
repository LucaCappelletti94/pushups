package rs.pushups.ci

import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.AfterClass
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assume.assumeTrue
import org.junit.BeforeClass
import org.junit.FixMethodOrder
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.MethodSorters

/** Exercises presentation refusal with one unavailable prerequisite per manifest variant. */
@RunWith(AndroidJUnit4::class)
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class PresentationRefusalsTest {

    companion object {
        private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
        private val context: Context get() = instrumentation.targetContext
        private val notifications: NotificationManager
            get() = context.getSystemService(NotificationManager::class.java)
        private val hasLauncher: Boolean
            get() = context.packageManager.getLaunchIntentForPackage(context.packageName) != null

        @BeforeClass
        @JvmStatic
        fun setUp() {
            if (Build.VERSION.SDK_INT >= 33 && !hasLauncher) {
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
        fun finish() {
            writeCoverage()
        }
    }

    @Test
    fun a01_aDeniedPermissionRefuses() {
        assumeTrue(Build.VERSION.SDK_INT >= 33 && hasLauncher)
        assertEquals(
            PackageManager.PERMISSION_DENIED,
            context.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS),
        )
        try {
            assertNotNull(Probe.present("pushups denied", "shown", null, null, null, null))
            assertEquals(0, notifications.activeNotifications.size)
        } finally {
            instrumentation.uiAutomation.grantRuntimePermission(
                context.packageName,
                android.Manifest.permission.POST_NOTIFICATIONS,
            )
        }
    }

    @Test
    fun a02_anIconUrlRefuses() {
        assumeTrue(hasLauncher)
        assertNotNull(Probe.present("pushups icon", "shown", null, null, "https://example.com/icon.png", null))
        assertEquals(0, notifications.activeNotifications.size)
    }

    @Test
    fun a03_aMissingLauncherRefuses() {
        assumeTrue(!hasLauncher)
        assertNotNull(Probe.present("pushups launcher", "shown", null, null, null, null))
        assertEquals(0, notifications.activeNotifications.size)
    }

    @Test
    fun a04_aDisabledChannelRefuses() {
        assumeTrue(Build.VERSION.SDK_INT >= 26 && hasLauncher)
        notifications.createNotificationChannel(
            NotificationChannel("pushups", "disabled", NotificationManager.IMPORTANCE_NONE),
        )
        assertNotNull(Probe.present("pushups channel", "shown", null, null, null, null))
        assertEquals(0, notifications.activeNotifications.size)
    }
}
