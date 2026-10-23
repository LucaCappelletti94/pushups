package rs.pushups.ci

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import rs.pushups.Native
import java.util.UUID

/** The generated export's failure policy, through the probe's own native controls. */
@RunWith(AndroidJUnit4::class)
class NativeBoundaryTest {
    @After
    fun finish() {
        Probe.setHandler()
        Native.onSession(false)
        writeCoverage()
    }

    @Test
    fun successReturnsRoundTripThroughNativeExports() {
        val id = "jni021-${UUID.randomUUID()}"
        assertTrue(Probe.boundaryTrue())
        assertNull(Probe.boundaryNull())
        assertEquals(id, Probe.boundaryObject(id))
        assertEquals("recovered $id", Probe.boundaryRecovery(id))
    }

    @Test
    fun aRustJniErrorThrowsAndRecovers() {
        val id = "jni021-${UUID.randomUUID()}"
        val error = runCatching { Probe.boundaryError() }.exceptionOrNull()
        assertTrue("id=$id error=$error", error is RuntimeException)
        assertEquals("recovered $id", Probe.boundaryRecovery(id))
    }

    @Test
    fun anOrdinaryPanicThrowsAndRecovers() {
        val id = "jni021-${UUID.randomUUID()}"
        val error = runCatching { Probe.boundaryPanic(id) }.exceptionOrNull()
        assertTrue("id=$id error=$error", error is RuntimeException)
        assertEquals("recovered $id", Probe.boundaryRecovery(id))
    }

    @Test
    fun aPendingExceptionSurvivesTheNextGeneratedEntry() {
        val id = "jni021-${UUID.randomUUID()}"
        assertNull(Probe.install())
        awaitNoActivity()
        Probe.setHandler()
        val error = runCatching { Probe.boundaryPending(id) }.exceptionOrNull()
        assertTrue("id=$id error=$error", error is IllegalStateException)
        assertTrue("id=$id error=$error", error?.message.orEmpty().contains(id))
        assertFalse(
            "id=$id got ${Probe.events()}",
            lines(Probe.events()).any { it.contains("boundary nested $id") },
        )
        assertEquals("recovered $id", Probe.boundaryRecovery(id))
    }

    @Test
    fun aPanicWithAPendingExceptionKeepsTheOriginal() {
        val id = "jni021-${UUID.randomUUID()}"
        val error = runCatching { Probe.boundaryPanicPending(id) }.exceptionOrNull()
        assertTrue("id=$id error=$error", error is IllegalStateException)
        assertTrue("id=$id error=$error", error?.message.orEmpty().contains(id))
        assertEquals("recovered $id", Probe.boundaryRecovery(id))
    }

    @Test
    fun aPanickingPayloadDropThrowsAndRecovers() {
        val id = "jni021-${UUID.randomUUID()}"
        val error = runCatching { Probe.boundaryPanickingDrop(id) }.exceptionOrNull()
        assertTrue("id=$id error=$error", error is RuntimeException)
        assertEquals("recovered $id", Probe.boundaryRecovery(id))
    }
}
