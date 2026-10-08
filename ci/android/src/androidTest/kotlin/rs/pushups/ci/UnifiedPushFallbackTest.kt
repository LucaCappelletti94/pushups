package rs.pushups.ci

import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.AfterClass
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith

/** An app with a `UnifiedPushConfig` on a device with no distributor, which run.sh arranges. */
@RunWith(AndroidJUnit4::class)
class UnifiedPushFallbackTest {

    companion object {
        @AfterClass
        @JvmStatic
        fun finish() = writeCoverage()
    }

    @Test
    fun registerFallsBackToFcm() {
        val vapid = InstrumentationRegistry.getArguments().getString("vapid")
            ?: error("run.sh passes the VAPID public key as the vapid argument")
        assertNull(Probe.installWithVapid(vapid))
        ActivityScenario.launch(TapActivity::class.java).use {
            Probe.setHandler()
            assertNull(Probe.register())
            awaitLine(Probe::events, "token ")
        }
    }
}
