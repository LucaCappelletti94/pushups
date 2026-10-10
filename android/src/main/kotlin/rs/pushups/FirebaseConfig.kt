package rs.pushups

/**
 * App-implemented through pushups-macros (firebase_config!). Returns
 * [applicationId, apiKey, gcmSenderId, projectId] for the matching client, or
 * null. Until the app opts in the symbol is missing and surfaces as an
 * UnsatisfiedLinkError.
 */
object FirebaseConfig {
    @JvmStatic
    external fun values(packageName: String): Array<String>?
}
