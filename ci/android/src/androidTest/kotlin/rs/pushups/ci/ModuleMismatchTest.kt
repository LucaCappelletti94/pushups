package rs.pushups.ci

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.AfterClass
import org.junit.Assert.assertEquals
import org.junit.FixMethodOrder
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.MethodSorters
import rs.pushups.Native
import java.util.UUID

/**
 * An app whose Kotlin module is another version than its Rust crate, as an AAR of one release
 * next to a crate of another would be. The provider stops at the version handshake, and `install`
 * names both versions instead of failing later in JNI. `run.sh` passes the crate's version as
 * `crate`.
 */
@RunWith(AndroidJUnit4::class)
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class ModuleMismatchTest {

    companion object {
        @AfterClass
        @JvmStatic
        fun finish() = writeCoverage()
    }

    @Test
    fun installNamesBothVersions() {
        val crate = InstrumentationRegistry.getArguments().getString("crate")
        assertEquals(
            "the pushups Android module is version 0.0.0-mismatch, but the app's pushups crate is $crate",
            Probe.install(),
        )
    }

    @Test
    fun zPushStillReachesTheHandlerWithoutTheRuntime() {
        val id = "mismatch-${UUID.randomUUID()}"
        Probe.setHandler()
        // The handshake established no runtime, so the push goes straight to the dispatcher.
        Native.onMessage(id.toByteArray(Charsets.UTF_8), false)
        assertEquals("message started_app=false $id", awaitLine(Probe::events, id, UI_MS))
    }
}
