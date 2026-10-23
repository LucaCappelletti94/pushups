package rs.pushups.ci

import android.app.ActivityManager
import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import android.os.Build
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry
import androidx.test.runner.lifecycle.Stage
import androidx.test.uiautomator.By
import androidx.test.uiautomator.StaleObjectException
import androidx.test.uiautomator.UiDevice
import org.junit.After
import org.junit.AfterClass
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.BeforeClass
import org.junit.FixMethodOrder
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.MethodSorters

/** Exercises headless Rust-to-JNI presentation and notification-shade launcher taps. */
@RunWith(AndroidJUnit4::class)
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class PresentationTest {

    companion object {
        private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
        private val context: Context get() = instrumentation.targetContext
        private val device: UiDevice get() = UiDevice.getInstance(instrumentation)
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
        fun finish() {
            writeCoverage()
        }

        private fun shown(): List<Pair<String?, String?>> =
            notifications.activeNotifications
                .filter { it.notification.flags and Notification.FLAG_GROUP_SUMMARY == 0 }
                .map { it.tag to it.notification.extras.getCharSequence(Notification.EXTRA_TITLE)?.toString() }

        private fun awaitShown(
            boundMs: Long = UI_MS,
            expected: (List<Pair<String?, String?>>) -> Boolean,
        ) {
            waitFor(boundMs) { if (expected(shown())) Unit else null }
                ?: error("the notifications did not settle within ${boundMs / 1000} s, got\n${shown()}")
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

        private fun tap(title: String, needle: String?, navigate: String? = null, boundMs: Long = DELIVERY_MS) {
            val monitor = instrumentation.addMonitor(TapActivity::class.java.name, null, false)
            try {
                device.openNotification()
                waitFor(boundMs) {
                    try {
                        device.findObject(By.text(title))?.click()
                    } catch (e: StaleObjectException) {
                        // The shade can replace a row while opening.
                    }
                    when {
                        needle != null -> lines(Probe.events()).firstOrNull { it.contains(needle) }
                        monitor.lastActivity != null -> "opened"
                        else -> null
                    }
                } ?: error("tapping $title did not deliver within ${boundMs / 1000} s, events ${Probe.events().toCharArray().map { it.code.toString(16) }}, activity ${monitor.lastActivity}")
                val activity = monitor.lastActivity
                if (navigate != null) {
                    assertEquals(navigate, activity?.intent?.dataString)
                }
                if (activity != null) {
                    instrumentation.runOnMainSync { activity.recreate() }
                    instrumentation.waitForIdleSync()
                }
            } finally {
                instrumentation.removeMonitor(monitor)
                closeUi()
            }
        }

        /** This process's importance, which the home press must lower. */
        private fun importance(): Int =
            ActivityManager.RunningAppProcessInfo().also { ActivityManager.getMyMemoryState(it) }.importance

        private val live = listOf(
            Stage.PRE_ON_CREATE,
            Stage.CREATED,
            Stage.STARTED,
            Stage.RESUMED,
            Stage.PAUSED,
            Stage.STOPPED,
            Stage.RESTARTED,
        )
    }
    @Before
    fun clearNotifications() {
        notifications.cancelAll()
        awaitShown { it.isEmpty() }
    }

    @After
    fun closePresentation() {
        notifications.cancelAll()
        closeUi()
    }


    @Test
    fun a01_aShowPostsWithoutAnActivity() {
        assertNull(Probe.present("pushups headless", "shown without", null, null, null, null))
        assertNull(Probe.present("pushups tagged", "under a tag", null, "dup", null, null))
        awaitShown { shown ->
            shown.size == 2 &&
                shown.firstOrNull { it.first == "dup" }?.second == "pushups tagged" &&
                shown.none { it.first == null }
        }
        assertFalse(activityAlive())
    }

    @Test
    fun a02_aTagReplacesItsNotificationAndUntaggedAreIndependent() {
        assertNull(Probe.present("pushups first", "independent", null, null, null, null))
        assertNull(Probe.present("pushups tagged", "under a tag", null, "dup", null, null))
        awaitShown { it.size == 2 }
        assertNull(Probe.present("pushups tagged two", "replacing", null, "dup", null, null))
        awaitShown { shown ->
            shown.size == 2 && shown.firstOrNull { it.first == "dup" }?.second == "pushups tagged two"
        }

        assertNull(Probe.present("pushups second", "another", null, null, null, null))
        awaitShown { shown -> shown.size == 3 && shown.count { it.first == "dup" } == 1 }
    }

    @Test
    fun a03_aColdTapDeliversTheDataOnceWithoutTheBackgroundHandler() {
        val data = "ciao — ünïcode ✓ 🔔 \u0000"
        val navigate = "pushups://notification/42?source=rendering"
        assertNull(Probe.present("pushups tap", "tap to open", navigate, "tap-cold", null, data))
        tap("pushups tap", data, navigate)
        val line = lines(Probe.events()).first { it.contains(data) }
        assertEquals("message started_app=true $data", line)
        assertEquals(1, lines(Probe.events()).count { it.contains(data) })
        assertEquals("", Probe.handled())
    }

    @Test
    fun a04_aTapWithoutDataLaunchesAlone() {
        assertNull(Probe.present("pushups alone", "launch only", null, "tap-alone", null, null))
        val before = lines(Probe.events()).size
        tap("pushups alone", null)
        assertEquals(before, lines(Probe.events()).size)
        assertEquals("", Probe.handled())
    }

    @Test
    fun a05_aTapWithEmptyDataDeliversTheEmptyPayload() {
        assertNull(Probe.present("pushups empty", "empty data", null, "tap-empty", null, ""))
        val before = lines(Probe.events()).size
        tap("pushups empty", "message started_app=false ")
        assertEquals(listOf("message started_app=false "), lines(Probe.events()).drop(before))
    }

    @Test
    fun a06_aTapIntoTheRunningSessionDeliversThroughResume() {
        instrumentation.startActivitySync(Intent(context, TapActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        device.pressHome()
        waitFor(UI_MS) {
            if (importance() != ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND) Unit else null
        } ?: error("the app was still in the foreground ${UI_MS / 1000} s after going home")
        val data = "live session — ünïcode"
        assertNull(Probe.present("pushups live", "tap to open", null, "tap-live", null, data))
        val before = lines(Probe.events()).size
        tap("pushups live", data)
        assertEquals(listOf("message started_app=false $data"), lines(Probe.events()).drop(before))
        assertEquals("", Probe.handled())
    }
}
