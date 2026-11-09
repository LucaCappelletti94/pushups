package rs.pushups.ci

import android.content.Context
import android.content.Intent
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.AfterClass
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.BeforeClass
import org.junit.FixMethodOrder
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.MethodSorters

/**
 * The tap's delivered-ids history: a persisted list is read once per process, an id in it is not
 * delivered again, the newest 64 ids are kept, and a non-string extra drops out of the payload.
 * Taps arrive as Activities carrying the FCM message id, as FCM's own notification taps do.
 */
@RunWith(AndroidJUnit4::class)
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class TapHistoryTest {

    companion object {
        private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
        private val context: Context get() = instrumentation.targetContext
        private const val MAX_IDS = 64

        @BeforeClass
        @JvmStatic
        fun setUp() {
            // A history from a previous process, the full 64 ids, so the window is at its bound.
            val ids = (0 until MAX_IDS).joinToString(",") { "\"old-%02d\"".format(it) }
            context.getSharedPreferences("pushups", Context.MODE_PRIVATE).edit()
                .putString("delivered_message_ids", "[${ids}]").commit()
            assertNull(Probe.install())
            Probe.setHandler()
        }

        @AfterClass
        @JvmStatic
        fun finish() = writeCoverage()

        /** Launches a tap carrying the FCM message id [id] and the optional extra. */
        private fun tap(id: String, extra: Pair<String, Any>? = null) {
            val intent = Intent(context, TapActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                .putExtra("google.message_id", id)
            extra?.let { (key, value) ->
                when (value) {
                    is String -> intent.putExtra(key, value)
                    is ByteArray -> intent.putExtra(key, value)
                }
            }
            instrumentation.startActivitySync(intent)
        }

        /** The events that are pushed, excluding tokens. */
        private fun messages(): List<String> = lines(Probe.events()).filter { it.startsWith("message ") }
    }

    @Test
    fun t01_anIdFromThePreviousProcessIsNotDeliveredAgain() {
        tap("old-63")
        assertEquals(0, messages().size)
    }

    @Test
    fun t02_aNewTapIsDeliveredAndTrimsTheOldestId() {
        tap("new-1", "id" to "n1")
        val line = awaitLine(Probe::events, "n1", UI_MS)
        assertTrue(line, line.startsWith("message started_app=false "))
        assertTrue(line, line.contains("""{"id":"n1"}"""))
    }

    @Test
    fun t03_theTrimmedOldestIdIsDeliveredAgain() {
        val before = messages().size
        tap("old-00")
        val line = waitFor(UI_MS) { if (messages().size > before) messages()[messages().size - 1] else null }
            ?: error("the trimmed id was not delivered within ${UI_MS / 1000} s, got:\n${Probe.events()}")
        assertEquals("message started_app=false {}", line)
    }

    @Test
    fun t04_aNonStringExtraDropsOutOfTheTapPayload() {
        val before = messages().size
        tap("new-2", "bytes" to byteArrayOf(1, 2, 3))
        val line = waitFor(UI_MS) { if (messages().size > before) messages()[messages().size - 1] else null }
            ?: error("the non-string-extra tap was not delivered within ${UI_MS / 1000} s, got:\n${Probe.events()}")
        assertEquals("message started_app=false {}", line)
    }
}
