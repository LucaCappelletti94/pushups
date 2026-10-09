package rs.pushups.ci

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.AfterClass
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

/**
 * An app whose Kotlin module is another version than its Rust crate, as an AAR of one release
 * next to a crate of another would be. The provider stops at the version handshake, and `install`
 * names both versions instead of failing later in JNI. `run.sh` passes the crate's version as
 * `crate`.
 */
@RunWith(AndroidJUnit4::class)
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
}
