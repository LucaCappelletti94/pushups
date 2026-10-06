package rs.pushups

import android.content.Context

/**
 * App-implemented through pushups-macros (background_handler). Until the app
 * declares a handler the symbol is missing and surfaces as an UnsatisfiedLinkError.
 */
object BackgroundHandler {
    @JvmStatic
    external fun handle(context: Context, payload: ByteArray, startedApp: Boolean)
}
