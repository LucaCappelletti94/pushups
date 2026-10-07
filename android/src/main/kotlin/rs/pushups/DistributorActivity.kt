package rs.pushups

import android.app.Activity
import android.os.Bundle
import org.unifiedpush.android.connector.UnifiedPush

/**
 * Chooses the UnifiedPush distributor once, since the connector's picker needs an Activity. On
 * success it registers through UnifiedPush, and on refusal it falls back to FCM.
 */
class DistributorActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val vapid = intent.getStringExtra(EXTRA_VAPID)
        if (vapid == null || savedInstanceState != null) {
            finish()
            return
        }
        UnifiedPush.tryUseDefaultDistributor(this) { chosen ->
            if (chosen) {
                Pushups.registerUnifiedPush(applicationContext, vapid)
            } else {
                Pushups.registerFcm(applicationContext)
            }
            finish()
        }
    }

    companion object {
        const val EXTRA_VAPID = "rs.pushups.VAPID"
    }
}
