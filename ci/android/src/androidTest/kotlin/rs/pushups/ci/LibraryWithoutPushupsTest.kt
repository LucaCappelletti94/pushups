package rs.pushups.ci

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.AfterClass
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

/**
 * An app whose named library has no `pushups` handshake, as a library built with a crate older
 * than the module has none. The provider leaves push off without calling any other export, and
 * `install`, from the probe the test loads itself, reports the module as never having started.
 */
@RunWith(AndroidJUnit4::class)
class LibraryWithoutPushupsTest {

    companion object {
        @AfterClass
        @JvmStatic
        fun finish() = writeCoverage()
    }

    @Test
    fun pushStaysOffAndInstallReportsTheModuleMissing() {
        System.loadLibrary("pushups_probe")
        assertEquals("the pushups Android module is not in the app", Probe.install())
    }
}
