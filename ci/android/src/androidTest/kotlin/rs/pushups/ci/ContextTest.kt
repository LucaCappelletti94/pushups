package rs.pushups.ci

import android.content.Context
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.AfterClass
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.BeforeClass
import org.junit.Test
import org.junit.runner.RunWith
import rs.pushups.deliverPush
import java.nio.charset.StandardCharsets
import java.util.UUID

/** `Context::with_jni` lends the exact context a push arrived on, and a global reference from it works on a worker. */
@RunWith(AndroidJUnit4::class)
class ContextTest {

    companion object {
        private val context: Context get() = InstrumentationRegistry.getInstrumentation().targetContext

        @BeforeClass
        @JvmStatic
        fun setUp() {
            assertNull(Probe.install())
        }

        @AfterClass
        @JvmStatic
        fun finish() = writeCoverage()
    }

    @Test
    fun theHandlerSeesTheDeviceProtectedContextItWasGiven() {
        val device = context.createDeviceProtectedStorageContext()
        val files = device.filesDir.path
        assertNotEquals(context.filesDir.path, files)
        val id = "context-${UUID.randomUUID()}"
        deliverPush(device, """{"context":"$id"}""".toByteArray(StandardCharsets.UTF_8))
        assertEquals("context $id files=$files worker=$files", awaitLine(Probe::handled, "context $id", UI_MS))
    }
}
