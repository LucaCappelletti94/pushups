package rs.pushups.ci

import android.os.SystemClock
import androidx.test.platform.app.InstrumentationRegistry
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

/** Writes the Rust coverage profile where run.sh pulls it from. */
fun writeCoverage() {
    val files = InstrumentationRegistry.getInstrumentation().targetContext.filesDir
    assertTrue(Probe.writeCoverage(File(files, "pushups.profraw").path))
}
