package rs.pushups.ci

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.AfterClass
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * An app whose Firebase configuration does not work: none compiled in, none for its package, or
 * one whose keys Firebase rejects. run.sh passes the text the failure must contain as `expect`.
 */
@RunWith(AndroidJUnit4::class)
class RegistrationFailsTest {

    companion object {
        @AfterClass
        @JvmStatic
        fun finish() = writeCoverage()
    }

    @Test
    fun registerReportsTheFailure() {
        val expected = InstrumentationRegistry.getArguments().getString("expect")
            ?: error("run.sh passes the expected failure as the expect argument")
        assertNull(Probe.install())
        Probe.setHandler()
        assertNull(Probe.register())
        val line = awaitLine(Probe::events, "failed ")
        assertTrue(line, line.contains(expected))
    }
}
