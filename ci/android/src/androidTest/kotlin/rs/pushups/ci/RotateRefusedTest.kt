package rs.pushups.ci

import android.content.Context
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.AfterClass
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.BeforeClass
import org.junit.Test
import org.junit.runner.RunWith
import rs.pushups.Transport

/**
 * Key rotation keeps the trait's refusal on a UnifiedPush transport, since the distributor owns
 * the app's keys. The test sets the transport the module would set after a UnifiedPush
 * registration and reads the refusal back.
 */
@RunWith(AndroidJUnit4::class)
class RotateRefusedTest {

    companion object {
        private val context: Context get() = InstrumentationRegistry.getInstrumentation().targetContext

        @BeforeClass
        @JvmStatic
        fun install() {
            assertNull(Probe.install())
            Probe.setHandler()
        }

        @AfterClass
        @JvmStatic
        fun finish() = writeCoverage()
    }

    @Test
    fun rotateKeysIsRefusedOnUnifiedPush() {
        Transport.set(context, Transport.UNIFIED_PUSH)
        assertEquals("push notifications are not supported on this target", Probe.rotateKeys())
    }
}
