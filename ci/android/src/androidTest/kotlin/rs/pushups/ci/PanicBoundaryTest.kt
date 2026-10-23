package rs.pushups.ci

import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import rs.pushups.Native
import java.util.UUID

@RunWith(AndroidJUnit4::class)
class PanicBoundaryTest {
    @After
    fun finish() {
        Probe.setHandler()
        Native.onSession(false)
        writeCoverage()
    }

    @Test
    fun aPanickingHandlerLeavesLaterEventsDeliverable() {
        val arguments = InstrumentationRegistry.getArguments()
        val kind = arguments.getString("panicCase") ?: error("panicCase is required")
        val id = arguments.getString("requestId") ?: "jni-$kind-${UUID.randomUUID()}"
        assertTrue(kind, kind in setOf("registered", "dropped", "unregistered", "session"))
        assertNull(Probe.install())
        awaitNoActivity()
        if (kind != "session") Native.onSession(true)
        Probe.setPanickingHandler(kind, id)
        if (kind == "session") {
            Native.onMessage(id.toByteArray(Charsets.UTF_8), false)
            assertFalse(Probe.events().contains(id))
        }

        Log.i("pushups-ci", "id=$id case=$kind phase=invoke")
        val error = runCatching {
            when (kind) {
                "registered" -> Native.onRegistered(id, null)
                "dropped" -> Native.onMessagesDropped()
                "unregistered" -> Native.onUnregistered()
                "session" -> Native.onSession(true)
            }
        }.exceptionOrNull()
        assertTrue("id=$id error=$error", error is RuntimeException)

        Probe.setHandler()
        val token = "$id-recovery-token"
        val message = "$id-recovery-message"
        Native.onRegistered(token, null)
        Native.onMessage(message.toByteArray(Charsets.UTF_8), false)
        assertEquals(
            listOf("token $token", "message started_app=false $message"),
            lines(Probe.events()),
        )
        Log.i("pushups-ci", "id=$id case=$kind phase=recovered")
    }
}
