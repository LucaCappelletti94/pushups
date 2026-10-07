package rs.pushups.ci

import android.content.Context
import android.os.SystemClock
import org.json.JSONObject
import java.io.File

/**
 * Asks the host to send an FCM message. The test writes `<id>.json` into `files/pushups-ci`, and
 * ci/android/send.py, polling the directory over adb, sends it with the service account key, which
 * never reaches the device, and answers in `<id>.status`.
 */
object Pushes {
    private const val ANSWER_MS = 60_000L

    /** Sends [message], an FCM HTTP v1 `message` object, and fails unless FCM accepted it. */
    fun send(context: Context, id: String, message: JSONObject) {
        val dir = File(context.filesDir, "pushups-ci").apply { mkdirs() }
        val partial = File(dir, "$id.part")
        partial.writeText(message.toString())
        check(partial.renameTo(File(dir, "$id.json"))) { "could not queue push $id" }
        val status = File(dir, "$id.status")
        val answer = waitFor(ANSWER_MS) { status.takeIf { it.exists() }?.readText() }
            ?: error("the host did not send push $id within ${ANSWER_MS / 1000} s")
        check(answer.startsWith("$id 200")) { "FCM refused push $id: $answer" }
    }
}

/** Polls [probe] until it returns non-null or [boundMs] passes on the monotonic clock. */
fun <T> waitFor(boundMs: Long, probe: () -> T?): T? {
    val deadline = SystemClock.elapsedRealtime() + boundMs
    while (true) {
        probe()?.let { return it }
        if (SystemClock.elapsedRealtime() >= deadline) return null
        SystemClock.sleep(250)
    }
}
