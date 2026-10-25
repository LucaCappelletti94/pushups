package rs.pushups

import android.content.Context

/**
 * App-implemented through pushups-macros (rendering_handler). Until the app
 * declares a handler the symbol is missing and surfaces as an UnsatisfiedLinkError.
 */
object RenderingHandler {
    @JvmStatic
    external fun render(context: Context, payload: ByteArray, startedApp: Boolean)
}
