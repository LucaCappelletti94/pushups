package rs.pushups

import android.app.Application
import android.content.ContentProvider
import android.content.Context
import android.content.ContentValues
import android.database.Cursor
import android.net.Uri
import android.content.pm.PackageManager
import android.os.SystemClock
import android.util.Log
import com.google.firebase.FirebaseApp
import com.google.firebase.FirebaseOptions

/**
 * Runs at process start, before any service. Loads the app's cdylib, hands the
 * Rust side the VM and application context, initialises Firebase from the
 * compiled-in client, and installs the activity callbacks.
 */
class PushupsProvider : ContentProvider() {

    override fun onCreate(): Boolean {
        val appContext = context?.applicationContext ?: return false
        Pushups.context = appContext
        val libName = resolveLibName(appContext)
        if (libName == null) {
            Log.w(TAG, "no native library name, pushups stays off")
            return true
        }
        if (!loadAndInit(appContext, libName)) return true
        initFirebase(appContext)
        (appContext as Application).registerActivityLifecycleCallbacks(ProcessState)
        return true
    }

    private fun resolveLibName(appContext: Context): String? {
        val info = try {
            appContext.packageManager.getPackageInfo(
                appContext.packageName,
                PackageManager.GET_ACTIVITIES or PackageManager.GET_META_DATA
            )
        } catch (e: PackageManager.NameNotFoundException) {
            Log.e(TAG, "package info", e)
            return null
        }
        info.applicationInfo?.metaData?.getString("pushups.lib_name")?.let { return it }
        return info.activities?.firstNotNullOfOrNull { it.metaData?.getString("android.app.lib_name") }
    }

    private fun loadAndInit(appContext: Context, libName: String): Boolean {
        var loaded = true
        try {
            val loadStart = SystemClock.elapsedRealtime()
            System.loadLibrary(libName)
            Log.d(TAG, "loaded lib$libName.so in ${SystemClock.elapsedRealtime() - loadStart} ms")
        } catch (e: UnsatisfiedLinkError) {
            Log.e(TAG, "load $libName", e)
            loaded = false
        }
        if (loaded) {
            try {
                Native.init(appContext, appContext.filesDir.absolutePath)
                ProcessState.loaded = true
            } catch (e: RuntimeException) {
                Log.e(TAG, "native init", e)
            }
        }
        return ProcessState.loaded
    }

    private fun initFirebase(appContext: Context) {
        try {
            val values = FirebaseConfig.values(appContext.packageName)
            if (values == null) {
                Log.i(TAG, "no firebase client matches ${appContext.packageName}")
            } else {
                FirebaseApp.initializeApp(
                    appContext,
                    FirebaseOptions.Builder()
                        .setApplicationId(values[0])
                        .setApiKey(values[1])
                        .setGcmSenderId(values[2])
                        .setProjectId(values[3])
                        .build()
                )
                Log.i(TAG, "firebase initialised for ${values[3]}")
            }
        } catch (e: UnsatisfiedLinkError) {
            Log.i(TAG, "FCM not configured")
        } catch (e: RuntimeException) {
            Log.e(TAG, "firebase configuration", e)
        }
    }

    override fun query(
        uri: Uri,
        projection: Array<out String>?,
        selection: String?,
        selectionArgs: Array<out String>?,
        sortOrder: String?,
    ): Cursor? = null

    override fun getType(uri: Uri): String? = null

    override fun insert(uri: Uri, values: ContentValues?): Uri? = null

    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int = 0

    override fun update(
        uri: Uri,
        values: ContentValues?,
        selection: String?,
        selectionArgs: Array<out String>?,
    ): Int = 0
}
