package rs.pushups.ci

import android.content.Context
import org.json.JSONObject
import java.io.File

/** A Web Push subscription as a server keeps it, the keys base64url encoded. */
data class WebPushSubscription(val endpoint: String, val p256dh: String, val auth: String)

/**
 * Asks the host to send a push. The test writes `<id>.json` into `files/pushups-ci`, and
 * ci/android/send.py, polling the directory over adb, sends it with keys that never reach the
 * device, and answers in `<id>.status`.
 */
object Pushes {
    /** Covers the host's own retries of a push service's transient answers. */
    private const val ANSWER_MS = 90_000L

    /** Sends [message], an FCM HTTP v1 `message` object, and fails unless FCM accepted it. */
    fun send(context: Context, id: String, message: JSONObject) = request(context, id, message)

    /** Sends [payload] to [subscription] as a Web Push, and fails unless the push service accepted it. */
    fun sendWebPush(context: Context, id: String, subscription: WebPushSubscription, payload: String) {
        val keys = JSONObject().put("p256dh", subscription.p256dh).put("auth", subscription.auth)
        val webPush = JSONObject().put("endpoint", subscription.endpoint).put("keys", keys)
        request(context, id, JSONObject().put("webpush", webPush).put("payload", payload))
    }

    private fun request(context: Context, id: String, body: JSONObject) {
        val dir = File(context.filesDir, "pushups-ci").apply { mkdirs() }
        val partial = File(dir, "$id.part")
        partial.writeText(body.toString())
        check(partial.renameTo(File(dir, "$id.json"))) { "could not queue push $id" }
        val status = File(dir, "$id.status")
        val answer = waitFor(ANSWER_MS) { status.takeIf { it.exists() }?.readText() }
            ?: error("the host did not send push $id within ${ANSWER_MS / 1000} s")
        check(answer.startsWith("$id 200")) { "the push service refused push $id: $answer" }
    }
}
