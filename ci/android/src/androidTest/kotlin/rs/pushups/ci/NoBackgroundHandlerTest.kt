package rs.pushups.ci

import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.google.android.gms.tasks.Tasks
import com.google.firebase.messaging.FirebaseMessaging
import org.json.JSONObject
import org.junit.AfterClass
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import java.util.UUID
import java.util.concurrent.TimeUnit

/** An app without `#[background_handler]`, whose pushes still reach the handler of its UI. */
@RunWith(AndroidJUnit4::class)
class NoBackgroundHandlerTest {

    companion object {
        @AfterClass
        @JvmStatic
        fun finish() = writeCoverage()
    }

    @Test
    fun aPushReachesTheUiHandler() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val token = Tasks.await(FirebaseMessaging.getInstance().token, 120, TimeUnit.SECONDS)
        assertNull(Probe.install())
        ActivityScenario.launch(TapActivity::class.java).use {
            Probe.setHandler()
            val id = "no-handler-${UUID.randomUUID()}"
            Pushes.send(context, id, JSONObject().put("token", token).put("data", JSONObject().put("id", id)))
            awaitLine(Probe::events, id)
            assertEquals("", Probe.handled())
        }
    }
}
