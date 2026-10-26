package rs.pushups.ci

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * The target of an explicit broadcast that takes the package out of the stopped state
 * `am instrument` leaves it in, since delivering to a manifest receiver clears that state
 * without creating an Activity.
 */
class UnstoppedReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) = Unit
}
