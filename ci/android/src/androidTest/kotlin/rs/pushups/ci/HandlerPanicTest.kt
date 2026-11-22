package rs.pushups.ci

import android.content.Context
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.AfterClass
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.BeforeClass
import org.junit.FixMethodOrder
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.MethodSorters
import java.io.File
import java.nio.charset.StandardCharsets
import java.util.UUID
import rs.pushups.deliverPush

/**
 * A handler that throws must stop neither the other handler nor the queue. The push stays in
 * the module's queue file for the next session, which drains it once a session is up.
 */
@RunWith(AndroidJUnit4::class)
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class HandlerPanicTest {

    companion object {
        private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
        private val context: Context get() = instrumentation.targetContext

        private lateinit var scenario: ActivityScenario<TapActivity>

        @BeforeClass
        @JvmStatic
        fun setUp() {
            assertNull(Probe.install())
            Probe.setHandler()
        }

        @AfterClass
        @JvmStatic
        fun finish() {
            scenario.close()
            writeCoverage()
        }

        /**
         * Bounded wait for the push's record to reach the module's queue file, which keeps it for
         * the next session. The module appends while its main thread drains, so the Looper is
         * pumped between polls.
         */
        private fun awaitQueued(id: String) {
            val queue = File(context.filesDir, "pushups-queue")
            waitFor(UI_MS) {
                instrumentation.waitForIdleSync()
                if (queue.exists() && queue.readBytes().decodeToString().contains(id)) true else null
            } ?: error("the push never reached the queue file within ${UI_MS / 1000} s")
        }
    }

    @Test
    fun a01_aPanickingRenderingHandlerStopsNeitherTheBackgroundHandlerNorTheQueue() {
        val id = "panic-render-${UUID.randomUUID()}"
        val payload = """{"panic-render":"$id"}"""
        deliverPush(context.applicationContext, payload.toByteArray(StandardCharsets.UTF_8))
        awaitLine(Probe::handled, id, UI_MS)
        assertFalse(Probe.rendered(), Probe.rendered().contains(id))
        awaitQueued(id)
        assertFalse(Probe.events().contains(id))
    }

    @Test
    fun a02_aPanickingBackgroundHandlerStopsNeitherTheRenderingHandlerNorTheQueue() {
        val id = "panic-bg-${UUID.randomUUID()}"
        val payload = """{"panic-bg":"$id"}"""
        deliverPush(context.applicationContext, payload.toByteArray(StandardCharsets.UTF_8))
        awaitLine(Probe::rendered, id, UI_MS)
        assertFalse(Probe.handled(), Probe.handled().contains(id))
        awaitQueued(id)
        assertFalse(Probe.events().contains(id))
    }

    @Test
    fun a03_theQueueDrainsBothPushesAndLaterOnesReachEveryJob() {
        scenario = ActivityScenario.launch(TapActivity::class.java)
        val events = lines(Probe.events())
        val renderedAt = events.indexOfFirst { it.contains("panic-render-") }
        val backgroundAt = events.indexOfFirst { it.contains("panic-bg-") }
        assertTrue(events.joinToString("\n"), renderedAt >= 0 && renderedAt < backgroundAt)
        assertTrue(events[renderedAt], events[renderedAt].startsWith("message started_app=true "))
        assertTrue(events[backgroundAt], events[backgroundAt].startsWith("message started_app=false "))

        val id = "after-${UUID.randomUUID()}"
        val payload = """{"after":"$id"}"""
        deliverPush(context.applicationContext, payload.toByteArray(StandardCharsets.UTF_8))
        awaitLine(Probe::events, id, UI_MS)
        awaitLine(Probe::rendered, id, UI_MS)
        awaitLine(Probe::handled, id, UI_MS)
        assertTrue(Probe.events(), 3 == lines(Probe.events()).size)
    }
}
