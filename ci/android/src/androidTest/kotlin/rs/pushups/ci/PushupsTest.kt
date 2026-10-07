package rs.pushups.ci

import android.app.Activity
import android.content.Context
import android.os.Build
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry
import androidx.test.runner.lifecycle.Stage
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import com.google.android.gms.tasks.Tasks
import com.google.firebase.messaging.FirebaseMessaging
import org.json.JSONObject
import org.junit.AfterClass
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.BeforeClass
import org.junit.FixMethodOrder
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.MethodSorters
import java.util.UUID
import java.util.concurrent.TimeUnit
import rs.pushups.PushupsMessagingService

/**
 * Walks one process through the plan's Android delivery table with real FCM pushes. The steps
 * share the process, so they run in name order. The runner finishes every Activity after each
 * step, which ends the UI session, so a step that needs a session opens its own.
 */
@RunWith(AndroidJUnit4::class)
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class PushupsTest {

    companion object {
        private const val UI_MS = 30_000L

        private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
        private val context: Context get() = instrumentation.targetContext
        private val device: UiDevice get() = UiDevice.getInstance(instrumentation)

        private lateinit var token: String
        private var scenario: ActivityScenario<TapActivity>? = null
        private val ids = mutableMapOf<String, String>()

        @BeforeClass
        @JvmStatic
        fun fetchToken() {
            token = Tasks.await(FirebaseMessaging.getInstance().token, 120, TimeUnit.SECONDS)
        }

        @AfterClass
        @JvmStatic
        fun finish() {
            scenario?.close()
            writeCoverage()
        }

        /** A fresh id for the push named [name], remembered for later steps. */
        private fun id(name: String): String =
            "$name-${UUID.randomUUID()}".also { ids[name] = it }

        private fun sendData(id: String) {
            Pushes.send(context, id, JSONObject().put("token", token).put("data", JSONObject().put("id", id)))
        }

        private fun openUi() {
            scenario = ActivityScenario.launch(TapActivity::class.java)
        }

        private fun closeUi() {
            scenario?.close()
            scenario = null
        }

        private fun awaitPermission(): String =
            waitFor(UI_MS) { Probe.permission() } ?: error("no permission answer within ${UI_MS / 1000} s")

        /** Answers the system prompt with the button whose resource id ends in [button]. */
        private fun answerPrompt(button: String) {
            val found = device.wait(Until.findObject(By.res("com.android.permissioncontroller", button)), UI_MS)
            assertNotNull("the permission prompt did not show $button", found)
            found.click()
        }
    }

    @Test
    fun a01_installFindsTheModule() {
        assertNull(Probe.install())
    }

    @Test
    fun a02_aHeadlessPushRunsTheBackgroundHandlerAndWaits() {
        val id = id("headless")
        sendData(id)
        val handled = awaitLine(Probe::handled, id)
        assertTrue(handled, handled.startsWith("message started_app=true "))
        assertFalse(Probe.events().contains(id))
    }

    @Test
    fun a03_aSessionQueuesUntilItsHandlerIsSetAndThenEmitsAtOnce() {
        openUi()
        val unbound = id("unbound")
        sendData(unbound)
        awaitLine(Probe::handled, unbound)
        assertFalse(Probe.events().contains(unbound))
        Probe.setHandler()
        val events = lines(Probe.events())
        val headlessAt = events.indexOfFirst { it.contains(ids.getValue("headless")) }
        val unboundAt = events.indexOfFirst { it.contains(unbound) }
        assertTrue(events.joinToString("\n"), headlessAt >= 0 && headlessAt < unboundAt)
        assertTrue(events[headlessAt], events[headlessAt].startsWith("message started_app=true "))
        assertTrue(events[unboundAt], events[unboundAt].startsWith("message started_app=false "))

        val bound = id("bound")
        sendData(bound)
        assertTrue(awaitLine(Probe::events, bound).startsWith("message started_app=false "))
        awaitLine(Probe::handled, bound)
    }

    @Test
    fun a04_registerReportsTheToken() {
        assertNull(Probe.register())
        awaitLine(Probe::events, "token $token")
    }

    @Test
    fun a05_thePromptAnswerIsReported() {
        if (Build.VERSION.SDK_INT < 33) {
            // Before Android 13 notifications need no runtime permission, so no prompt shows.
            Probe.startPermission()
            assertEquals("Granted", awaitPermission())
            return
        }
        Probe.startPermission()
        answerPrompt("permission_deny_button")
        assertEquals("Denied", awaitPermission())
        Probe.startPermission()
        answerPrompt("permission_allow_button")
        assertEquals("Granted", awaitPermission())
        // Granted already, so no prompt shows.
        Probe.startPermission()
        assertEquals("Granted", awaitPermission())
    }

    @Test
    fun a06_endingTheSessionQueuesAgainForTheNextOne() {
        openUi()
        Probe.setHandler()
        closeUi()
        val id = id("ended")
        sendData(id)
        awaitLine(Probe::handled, id)
        assertFalse(Probe.events().contains(id))
        openUi()
        Probe.setHandler()
        assertTrue(awaitLine(Probe::events, id, UI_MS).startsWith("message started_app=false "))
    }

    @Test
    fun a07_aTapDeliversTheDataOnce() {
        closeUi()
        // A corrupt tap history starts afresh, and the tap still arrives.
        context.getSharedPreferences("pushups", Context.MODE_PRIVATE).edit()
            .putString("delivered_message_ids", "not a list").commit()
        val id = id("tap")
        val title = "pushups tap $id"
        Pushes.send(
            context,
            id,
            JSONObject()
                .put("token", token)
                .put("notification", JSONObject().put("title", title).put("body", "tap to open"))
                .put("data", JSONObject().put("id", id))
                .put("android", JSONObject().put("notification", JSONObject().put("click_action", "rs.pushups.ci.TAP"))),
        )
        val monitor = instrumentation.addMonitor(TapActivity::class.java.name, null, false)
        device.openNotification()
        val notification = device.wait(Until.findObject(By.text(title)), DELIVERY_MS)
        assertNotNull("the notification $title did not show", notification)
        notification.click()
        val activity: Activity = instrumentation.waitForMonitorWithTimeout(monitor, UI_MS)
            ?: error("the tap did not open TapActivity")
        Probe.setHandler()
        val line = awaitLine(Probe::events, id, UI_MS)
        assertTrue(line, line.startsWith("message started_app=false "))
        assertFalse(line, line.contains("google."))
        // A recreated Activity keeps the tap's intent, and the tap must not arrive twice.
        instrumentation.runOnMainSync { activity.recreate() }
        instrumentation.waitForIdleSync()
        assertEquals(1, lines(Probe.events()).count { it.contains(id) })
        instrumentation.runOnMainSync {
            ActivityLifecycleMonitorRegistry.getInstance().getActivitiesInStage(Stage.RESUMED).forEach { it.finish() }
        }
    }

    @Test
    fun a08_droppedMessagesReachTheHandler() {
        openUi()
        Probe.setHandler()
        // FCM calls this after discarding pushes it held too long, which no test can provoke.
        PushupsMessagingService().onDeletedMessages()
        awaitLine(Probe::events, "dropped", UI_MS)
    }
}
