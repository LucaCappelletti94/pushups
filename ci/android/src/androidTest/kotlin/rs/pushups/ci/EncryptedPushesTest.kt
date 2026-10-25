package rs.pushups.ci

import android.content.Context
import android.os.Build
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.google.android.gms.tasks.Tasks
import com.google.firebase.messaging.FirebaseMessaging
import org.json.JSONObject
import org.junit.AfterClass
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.BeforeClass
import org.junit.FixMethodOrder
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.MethodSorters
import java.util.UUID
import java.util.concurrent.TimeUnit

/**
 * The plan's Android encrypted push lines with real FCM pushes: the token reports the keys its
 * envelopes are sealed against, an encrypted push reaches every job as the plaintext, in the UI
 * and headless, a corrupt one reaches none, and an unencrypted push arrives as the data map.
 * The steps share the process, so they run in name order.
 */
@RunWith(AndroidJUnit4::class)
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class EncryptedPushesTest {

    companion object {
        private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
        private val context: Context get() = instrumentation.targetContext

        private lateinit var token: String
        private lateinit var p256dh: String
        private lateinit var auth: String
        private var scenario: ActivityScenario<TapActivity>? = null

        @BeforeClass
        @JvmStatic
        fun setup() {
            if (Build.VERSION.SDK_INT >= 33) {
                instrumentation.uiAutomation.grantRuntimePermission(
                    context.packageName,
                    android.Manifest.permission.POST_NOTIFICATIONS,
                )
            }
            token = Tasks.await(FirebaseMessaging.getInstance().token, 120, TimeUnit.SECONDS)
            assertNull(Probe.install())
            Probe.setHandler()
            assertNull(Probe.register())
        }

        @AfterClass
        @JvmStatic
        fun finish() {
            scenario?.close()
            writeCoverage()
        }

        /** The FCM message object of a data push to the token. */
        private fun message(id: String): JSONObject =
            JSONObject().put("token", token).put("data", JSONObject().put("id", id))

        /** Waits for a token line whose keys differ from [p256dh] and [auth] and remembers them. */
        private fun awaitNewKeys(): String {
            val fresh = waitFor(DELIVERY_MS) {
                lines(Probe.events())
                    .filter { it.startsWith("token $token p256dh=") }
                    .map { it.substringAfter("p256dh=").substringBefore(" auth=") to it.substringAfter(" auth=") }
                    .lastOrNull { it.first != p256dh || it.second != auth }
            } ?: error("the rotated token's keys did not differ within ${DELIVERY_MS / 1000} s, got:\n${Probe.events()}")
            p256dh = fresh.first
            auth = fresh.second
            return fresh.first
        }

        /** Checks that [line] is a job's push line for the plaintext [plaintext]. */
        private fun assertPlaintextLine(line: String, plaintext: String) {
            assertTrue(line, line.startsWith("message started_app="))
            assertTrue(line, line.contains(" $plaintext"))
            assertFalse(line, line.contains("\"pushups\""))
        }

        private fun openUi() {
            scenario = ActivityScenario.launch(TapActivity::class.java)
        }

        /** Ends the UI session. */
        private fun closeUi() {
            scenario?.close()
            scenario = null
            awaitNoActivity()
        }
    }

    @Test
    fun a01_registerReportsTheTokenWithItsKeys() {
        val line = awaitLine(Probe::events, "token $token p256dh=")
        p256dh = line.substringAfter("p256dh=").substringBefore(" auth=")
        auth = line.substringAfter(" auth=")
        assertTrue(line, p256dh.isNotEmpty() && auth.isNotEmpty())
    }

    @Test
    fun a02_anEncryptedPushReachesEveryJobInTheUi() {
        openUi()
        val id = "ui-${UUID.randomUUID()}"
        val plaintext = """{"encrypted":"$id"}"""
        Pushes.sendEncrypted(context, id, message(id), p256dh, auth, plaintext)
        assertPlaintextLine(awaitLine(Probe::events, id), plaintext)
        assertPlaintextLine(awaitLine(Probe::rendered, id), plaintext)
        assertPlaintextLine(awaitLine(Probe::handled, id), plaintext)
    }

    @Test
    fun a03_anEncryptedPushReachesEveryJobHeadless() {
        closeUi()
        val id = "headless-${UUID.randomUUID()}"
        val plaintext = """{"encrypted":"$id"}"""
        Pushes.sendEncrypted(context, id, message(id), p256dh, auth, plaintext)
        assertPlaintextLine(awaitLine(Probe::events, id), plaintext)
        assertPlaintextLine(awaitLine(Probe::rendered, id), plaintext)
        assertPlaintextLine(awaitLine(Probe::handled, id), plaintext)
    }

    @Test
    fun a04_aCorruptEnvelopeReachesNoJob() {
        val id = "corrupt-${UUID.randomUUID()}"
        Pushes.sendEncrypted(context, id, message(id), p256dh, auth, """{"encrypted":"$id"}""", corrupt = true)
        val arrived = waitFor(5_000) {
            if (Probe.events().contains(id) || Probe.rendered().contains(id) || Probe.handled().contains(id)) {
                "the corrupt envelope reached a job"
            } else {
                null
            }
        }
        assertNull(Probe.events(), arrived)
        assertNull(Probe.rendered(), arrived)
        assertNull(Probe.handled(), arrived)
    }

    @Test
    fun a05_anUnencryptedPushArrivesAsTheDataMap() {
        val id = "plain-${UUID.randomUUID()}"
        Pushes.send(context, id, message(id))
        val line = awaitLine(Probe::events, id)
        assertTrue(line, line.startsWith("message started_app="))
        assertTrue(line, line.contains("""{"id":"$id"}"""))
    }

    @Test
    fun a06_rotatedKeysReportAgainAndOpenTheNewPushes() {
        val old = p256dh
        assertNull(Probe.rotateKeys())
        val fresh = awaitNewKeys()
        assertTrue(fresh, fresh != old)
        val id = "rotated-${UUID.randomUUID()}"
        val plaintext = """{"encrypted":"$id"}"""
        Pushes.sendEncrypted(context, id, message(id), p256dh, auth, plaintext)
        assertPlaintextLine(awaitLine(Probe::events, id), plaintext)
        assertPlaintextLine(awaitLine(Probe::rendered, id), plaintext)
        assertPlaintextLine(awaitLine(Probe::handled, id), plaintext)
    }
}
