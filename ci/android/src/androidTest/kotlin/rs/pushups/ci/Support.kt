package rs.pushups.ci

import android.os.SystemClock
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry
import androidx.test.runner.lifecycle.Stage
import org.junit.Assert.assertTrue
import java.io.File

/** Polls [probe] until it returns non-null or [boundMs] passes on the monotonic clock. */
fun <T> waitFor(boundMs: Long, probe: () -> T?): T? {
    val deadline = SystemClock.elapsedRealtime() + boundMs
    while (true) {
        probe()?.let { return it }
        if (SystemClock.elapsedRealtime() >= deadline) return null
        SystemClock.sleep(250)
    }
}

/** The lines of one of the probe's logs. */
fun lines(text: String): List<String> = if (text.isEmpty()) emptyList() else text.split("\n")

/** How long a push may take to arrive, generous for a cold emulator. */
const val DELIVERY_MS = 90_000L

/** The first line of [source] containing [needle], waiting for it up to [boundMs]. */
fun awaitLine(source: () -> String, needle: String, boundMs: Long = DELIVERY_MS): String =
    waitFor(boundMs) { lines(source()).firstOrNull { it.contains(needle) } }
        ?: error("no line with $needle within ${boundMs / 1000} s, got:\n${source()}")

/** How long the UI may take to open or close, or a prompt to show. */
const val UI_MS = 30_000L

/**
 * Waits until no Activity of the process lives, which ends the UI session. The session lasts while
 * any lives, the test framework's own launch Activities included.
 */
fun awaitNoActivity() {
    val live = listOf(Stage.PRE_ON_CREATE, Stage.CREATED, Stage.STARTED, Stage.RESUMED, Stage.PAUSED, Stage.STOPPED, Stage.RESTARTED)
    waitFor(UI_MS) {
        var alive = 0
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            val monitor = ActivityLifecycleMonitorRegistry.getInstance()
            alive = live.sumOf { monitor.getActivitiesInStage(it).size }
        }
        if (alive == 0) Unit else null
    } ?: error("an Activity was still alive ${UI_MS / 1000} s after closing the UI")
}

/** Writes the Rust coverage profile where run.sh pulls it from. */
fun writeCoverage() {
    val files = InstrumentationRegistry.getInstrumentation().targetContext.filesDir
    assertTrue(Probe.writeCoverage(File(files, "pushups.profraw").path))
}
