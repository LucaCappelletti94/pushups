package rs.pushups.ci

import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.os.Build
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry
import androidx.test.runner.lifecycle.Stage
import androidx.test.uiautomator.By
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import org.junit.AfterClass
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.BeforeClass
import org.junit.Test
import org.junit.runner.RunWith
import rs.pushups.deliverPush
import java.nio.charset.StandardCharsets
import java.util.UUID

/** A rendering handler presents a push's notification before the background handler runs. */
@RunWith(AndroidJUnit4::class)
class RenderingHandlerTest {

    companion object {
        private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
        private val context: Context get() = instrumentation.targetContext
        private val device: UiDevice get() = UiDevice.getInstance(instrumentation)
        private val notifications: NotificationManager
            get() = context.getSystemService(NotificationManager::class.java)
        private val live = listOf(
            Stage.PRE_ON_CREATE,
            Stage.CREATED,
            Stage.STARTED,
            Stage.RESUMED,
            Stage.PAUSED,
            Stage.STOPPED,
            Stage.RESTARTED,
        )

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
        fun finish() {
            writeCoverage()
        }

        private fun activityAlive(): Boolean {
            var alive = false
            instrumentation.runOnMainSync {
                val monitor = ActivityLifecycleMonitorRegistry.getInstance()
                alive = live.any { monitor.getActivitiesInStage(it).isNotEmpty() }
            }
            return alive
        }

        private fun closeUi() {
            if (activityAlive()) {
                instrumentation.runOnMainSync {
                    val monitor = ActivityLifecycleMonitorRegistry.getInstance()
                    live.forEach { stage -> monitor.getActivitiesInStage(stage).forEach { it.finish() } }
                }
                instrumentation.waitForIdleSync()
            }
            awaitNoActivity()
        }

        /** The module's active user notifications, as tag and title. */
        private fun shown(): List<Pair<String?, String?>> =
            notifications.activeNotifications
                .filter { it.notification.flags and Notification.FLAG_GROUP_SUMMARY == 0 }
                .map { it.tag to it.notification.extras.getCharSequence(Notification.EXTRA_TITLE)?.toString() }
    }

    @Test
    fun aPushIsRenderedThroughTheRenderingHandlerAndTheTapDeliversItsDataOnce() {
        awaitNoActivity()
        val id = "proof-${UUID.randomUUID()}"
        val payload = """{"rendered":"$id"}"""
        val title = "pushups $id"
        deliverPush(context.applicationContext, payload.toByteArray(StandardCharsets.UTF_8))
        val rendered = "message started_app=true $payload"
        awaitLine(Probe::rendered, payload, UI_MS)
        assertEquals(listOf(rendered), lines(Probe.rendered()))
        waitFor(UI_MS) {
            val shown = shown()
            if (shown.size == 1 && shown[0].first == id && shown[0].second == title) Unit else null
        } ?: error("the rendering handler's notification did not show within ${UI_MS / 1000} s, notifications\n${shown()}\nrendered\n${Probe.rendered()}")
        assertTrue(Probe.handled().isEmpty())
        assertFalse(activityAlive())
        val tap = "message started_app=false $payload"
        val monitor = instrumentation.addMonitor(TapActivity::class.java.name, null, false)
        try {
            device.openNotification()
            waitFor(DELIVERY_MS) {
                try {
                    device.findObject(By.text(title))?.click()
                } catch (e: StaleObjectException) {
                    // The shade can replace a row while opening.
                }
                val events = lines(Probe.events())
                if (events.contains(rendered) && events.contains(tap) && monitor.lastActivity != null) "delivered" else null
            } ?: error("tapping $title never delivered the replay and the tap within ${DELIVERY_MS / 1000} s, events\n${Probe.events()}")
            val activity = checkNotNull(monitor.lastActivity) { "tapping $title never opened an Activity" }
            instrumentation.runOnMainSync { activity.recreate() }
            instrumentation.waitForIdleSync()
            assertEquals(listOf(rendered, tap), lines(Probe.events()).filter { it.contains(payload) })
        } finally {
            instrumentation.removeMonitor(monitor)
            closeUi()
        }
    }
}
