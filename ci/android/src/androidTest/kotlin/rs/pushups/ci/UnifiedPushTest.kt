package rs.pushups.ci

import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.AfterClass
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.FixMethodOrder
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.MethodSorters
import org.unifiedpush.android.connector.FailedReason
import org.unifiedpush.android.connector.data.PublicKeySet
import org.unifiedpush.android.connector.data.PushEndpoint
import org.unifiedpush.android.connector.data.PushMessage
import rs.pushups.PushupsMessagingService
import rs.pushups.PushupsPushService
import java.util.UUID

/**
 * An app with a `UnifiedPushConfig` on a device with a distributor, ntfy in run.sh, whose
 * endpoints start with the `endpoint` argument. Pushes come from the host's `web-push` sender.
 */
@RunWith(AndroidJUnit4::class)
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class UnifiedPushTest {

    companion object {
        private val context get() = InstrumentationRegistry.getInstrumentation().targetContext
        private lateinit var subscription: WebPushSubscription

        @AfterClass
        @JvmStatic
        fun finish() = writeCoverage()
    }

    @Test
    fun a1_registerChoosesTheDistributorAndReportsAWebPushToken() {
        val arguments = InstrumentationRegistry.getArguments()
        val vapid = arguments.getString("vapid") ?: error("run.sh passes the VAPID public key as the vapid argument")
        val prefix = arguments.getString("endpoint") ?: error("run.sh passes the distributor's endpoint prefix as the endpoint argument")
        assertNull(Probe.installWithVapid(vapid))
        // With no UI session the endpoint waits in the queue, so the token is awaited with one open.
        ActivityScenario.launch(TapActivity::class.java).use {
            Probe.setHandler()
            assertNull(Probe.register())
            val line = awaitLine(Probe::events, "webpush ")
            val (_, endpoint, p256dh, auth) = line.split(" ")
            assertTrue(line, endpoint.startsWith(prefix))
            subscription = WebPushSubscription(endpoint, p256dh, auth)
        }
    }

    @Test
    fun a2_aPushReachesTheHandlerAndTheBackgroundHandler() {
        ActivityScenario.launch(TapActivity::class.java).use {
            Probe.setHandler()
            val id = "unifiedpush-${UUID.randomUUID()}"
            Pushes.sendWebPush(context, id, subscription, """{"id":"$id"}""")
            val line = awaitLine(Probe::events, id)
            assertTrue(line, line.startsWith("message started_app=false "))
            awaitLine(Probe::handled, id)
        }
    }

    @Test
    fun a3_anFcmTokenIsDroppedWhileUnifiedPushCarriesThePushes() {
        ActivityScenario.launch(TapActivity::class.java).use {
            Probe.setHandler()
            // Firebase makes a token at start on its own, and the app keeps one token, its UnifiedPush one.
            PushupsMessagingService().onNewToken("fcm-token-that-must-not-arrive")
            // Delivered in order after the token, so once it shows a forwarded token would have too.
            PushupsMessagingService().onDeletedMessages()
            awaitLine(Probe::events, "dropped")
            assertFalse(Probe.events().contains("fcm-token-that-must-not-arrive"))
        }
    }

    @Test
    fun a4_aPushWithNoUiRunsTheBackgroundHandlerAndWaitsForTheNextSession() {
        awaitNoActivity()
        val id = "unifiedpush-headless-${UUID.randomUUID()}"
        Pushes.sendWebPush(context, id, subscription, """{"id":"$id"}""")
        awaitLine(Probe::handled, id)
        assertFalse(Probe.events().contains(id))
        ActivityScenario.launch(TapActivity::class.java).use {
            Probe.setHandler()
            awaitLine(Probe::events, id, UI_MS)
        }
    }

    @Test
    fun a5_theDistributorsFailuresReachTheHandler() {
        // No distributor fails on request, so the connector's callbacks are called as it would.
        val service = PushupsPushService()
        val badKeys = "the distributor's endpoint has no valid Web Push keys"
        // A distributor renews endpoints while no UI runs, so a bad one waits for the next session.
        awaitNoActivity()
        service.onNewEndpoint(PushEndpoint("https://ntfy.sh/upNoKeys?up=1", null), "default")
        service.onNewEndpoint(PushEndpoint("https://ntfy.sh/upShort?up=1", PublicKeySet("AAAA", "AAAA")), "default")
        assertFalse(Probe.events().contains(badKeys))
        ActivityScenario.launch(TapActivity::class.java).use {
            Probe.setHandler()
            assertEquals(Probe.events(), 2, lines(Probe.events()).count { it.contains(badKeys) })
            service.onRegistrationFailed(FailedReason.NETWORK, "default")
            awaitLine(Probe::events, "the distributor refused the registration: NETWORK", UI_MS)
            val undecrypted = "undecrypted-${UUID.randomUUID()}"
            service.onMessage(PushMessage(undecrypted.toByteArray(), false), "default")
            service.onUnregistered("default")
            awaitLine(Probe::events, "the distributor unregistered the app", UI_MS)
            assertFalse(Probe.events().contains(undecrypted))
        }
    }
}
