package rs.pushups.ci

import android.app.Activity
import android.content.Intent
import androidx.activity.ComponentActivity
import java.util.concurrent.atomic.AtomicInteger

/**
 * The UI the tests open and close to move the process between UI sessions, and a tap's target. A
 * plain Activity that keeps the newest intent with `setIntent`, as such apps should.
 */
class TapActivity : Activity() {
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        newIntents.incrementAndGet()
    }

    companion object {
        /** How many intents reached a running instance, which only a `singleTop` declaration allows. */
        val newIntents = AtomicInteger()
    }
}

/** An androidx Activity, as wry's is, that leaves `getIntent()` at the intent that created it. */
class ComponentTapActivity : ComponentActivity() {
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        newIntents.incrementAndGet()
    }

    companion object {
        /** How many intents reached a running instance. */
        val newIntents = AtomicInteger()
    }
}
