package rs.pushups.ci

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.AfterClass
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith

/**
 * An app that names its library only through an Activity's `android.app.lib_name`, as `dx` apps do.
 * The provider still loads it and initialises Firebase, so registration works.
 */
@RunWith(AndroidJUnit4::class)
class FallbackNameTest {

    companion object {
        @AfterClass
        @JvmStatic
        fun finish() = writeCoverage()
    }

    @Test
    fun theActivityNameLoadsTheLibrary() {
        assertNull(Probe.install())
        Probe.setHandler()
        assertNull(Probe.register())
        awaitLine(Probe::events, "token ")
    }
}
