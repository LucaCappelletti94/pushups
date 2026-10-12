package rs.pushups.ci

import android.app.Activity
import android.app.ActivityManager
import android.content.Intent
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.json.JSONObject
import org.junit.AfterClass
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.BeforeClass
import org.junit.FixMethodOrder
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.MethodSorters
import java.util.UUID
import java.util.concurrent.atomic.AtomicInteger

/**
 * Taps on a notification while a `singleTop` Activity runs, which reach its `onNewIntent` and
 * create nothing, as the Android tap table lays out.
 */
@RunWith(AndroidJUnit4::class)
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class SingleTopTapTest {

    companion object {
        private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
        private lateinit var token: String

        @BeforeClass
        @JvmStatic
        fun register() {
            // A fresh install may not post notifications from Android 13 on, and these tests are not about the prompt.
            if (android.os.Build.VERSION.SDK_INT >= 33) {
                instrumentation.uiAutomation.grantRuntimePermission(
                    instrumentation.targetContext.packageName,
                    android.Manifest.permission.POST_NOTIFICATIONS,
                )
            }
            assertNull(Probe.install())
            ActivityScenario.launch(TapActivity::class.java).use {
                Probe.setHandler()
                assertNull(Probe.register())
                token = awaitLine(Probe::events, "token ").removePrefix("token ")
            }
        }

        @AfterClass
        @JvmStatic
        fun finish() = writeCoverage()
    }

    @Test
    fun a1_aPlainActivityThatKeepsTheIntentGetsTheTapOnResume() {
        tapInto(TapActivity::class.java, "rs.pushups.ci.TAP", TapActivity.newIntents)
    }

    @Test
    fun a2_anAndroidxActivityGetsTheTapThroughItsListener() {
        tapInto(ComponentTapActivity::class.java, "rs.pushups.ci.TAP_COMPONENT", ComponentTapActivity.newIntents)
    }

    /** Opens [activity], sends it to the background alive, taps a notification aimed at it, and awaits the tap. */
    private fun <A : Activity> tapInto(activity: Class<A>, action: String, newIntents: AtomicInteger) {
        val device = UiDevice.getInstance(instrumentation)
        val context = instrumentation.targetContext
        // ActivityScenario loses an Activity whose intent changes, so the test holds the instance itself.
        val running = instrumentation.startActivitySync(
            Intent(context, activity).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        )
        try {
            Probe.setHandler()
            val id = "single-top-${UUID.randomUUID()}"
            val title = "pushups tap $id"
            // FCM shows a notification only while the process's importance is below foreground, which lags the home press, and the Activity stays alive there.
            device.pressHome()
            waitFor(UI_MS) { if (importance() != ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND) Unit else null }
                ?: error("the app was still in the foreground ${UI_MS / 1000} s after going home")
            Pushes.send(
                context,
                id,
                JSONObject()
                    .put("token", token)
                    .put("notification", JSONObject().put("title", title).put("body", "tap to open"))
                    .put("data", JSONObject().put("id", id))
                    .put("android", JSONObject().put("notification", JSONObject().put("click_action", action))),
            )
            val before = newIntents.get()
            device.openNotification()
            val tapped = waitFor(DELIVERY_MS) {
                device.wait(Until.findObject(By.text(title)), 1_000)?.click()
                if (newIntents.get() > before) Unit else null
            }
            checkNotNull(tapped) { "tapping $title never reached the running Activity's onNewIntent" }
            awaitLine(Probe::events, id, UI_MS)
            // A later resume with the same intent must not deliver the tap again. Android 10 bars a test from bringing its own task back, so Instrumentation drives the resume, whose onResume runs the app's onActivityResumed.
            instrumentation.runOnMainSync {
                instrumentation.callActivityOnPause(running)
                instrumentation.callActivityOnResume(running)
            }
            instrumentation.waitForIdleSync()
            assertEquals(1, lines(Probe.events()).count { line -> line.contains(id) })
        } finally {
            instrumentation.runOnMainSync { running.finish() }
        }
    }

    /** This process's importance, which FCM reads to choose between showing a push and handing it to the app. */
    private fun importance(): Int =
        ActivityManager.RunningAppProcessInfo().also { ActivityManager.getMyMemoryState(it) }.importance
}
