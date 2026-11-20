package rs.pushups.ci

import android.content.Context
import android.content.Intent
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.AfterClass
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.BeforeClass
import org.junit.FixMethodOrder
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.MethodSorters
import rs.pushups.DistributorActivity
import rs.pushups.Pushups
import rs.pushups.Transport

/**
 * The module's distributor chooser, launched the way the module launches it. Without the VAPID
 * key it closes without registering, and a refusal falls back to FCM.
 */
@RunWith(AndroidJUnit4::class)
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class DistributorRefusalTest {

    companion object {
        private val context: Context get() = InstrumentationRegistry.getInstrumentation().targetContext

        @BeforeClass
        @JvmStatic
        fun setUp() {
            assertNull(Probe.install())
            Probe.setHandler()
        }

        @AfterClass
        @JvmStatic
        fun finish() = writeCoverage()
    }

    @Test
    fun a01_anIntentWithoutTheVapidKeyClosesWithoutRegistering() {
        ActivityScenario.launch<DistributorActivity>(Intent(context, DistributorActivity::class.java)).close()
        awaitNoActivity()
        assertEquals(Transport.FCM, Pushups.transport())
    }

    @Test
    fun a02_aRefusalFallsBackToFcm() {
        val vapid = InstrumentationRegistry.getArguments().getString("vapid")
            ?: error("run.sh passes the VAPID public key as the vapid argument")
        ActivityScenario.launch<DistributorActivity>(
            Intent(context, DistributorActivity::class.java)
                .putExtra(DistributorActivity.EXTRA_VAPID, vapid),
        ).close()
        // The refusal's FCM token waits in the queue while no UI session is open, so the drain needs one.
        ActivityScenario.launch(TapActivity::class.java).use {
            awaitLine(Probe::events, "token ")
        }
        assertEquals(Transport.FCM, Pushups.transport())
    }
}
