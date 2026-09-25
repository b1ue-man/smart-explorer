package app.smartexplorer.android.update

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.util.Log
import androidx.core.content.FileProvider
import app.smartexplorer.android.system.Permissions
import java.io.File
import java.io.IOException

/**
 * Hands a downloaded update APK to the system installer (android-apis.md §7.1): `ACTION_VIEW` on a
 * content URI of the androidx FileProvider `${applicationId}.files` (cache-path `update/`).
 */
internal object UpdateInstaller {
    private const val TAG = "SmartExplorerUpdate"
    private const val APK_MIME = "application/vnd.android.package-archive"
    private const val UPDATE_DIR = "update"

    enum class Outcome { Started, NeedsPermission, NoInstaller, InvalidFile }

    /**
     * Starts the installer for [path]. Only a regular `.apk` file below `<cache>/update/` is
     * accepted, so nothing else can be exposed through the provider.
     */
    fun install(context: Context, path: String): Outcome {
        if (!Permissions.canInstallPackages(context)) return Outcome.NeedsPermission
        val file = apkFile(context, path) ?: return Outcome.InvalidFile
        val uri = try {
            FileProvider.getUriForFile(context, context.packageName + ".files", file)
        } catch (e: IllegalArgumentException) {
            Log.w(TAG, "update APK outside the provider paths: $path", e)
            return Outcome.InvalidFile
        }
        val intent = Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(uri, APK_MIME)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            if (context !is Activity) addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        return try {
            context.startActivity(intent)
            Outcome.Started
        } catch (e: ActivityNotFoundException) {
            Outcome.NoInstaller
        }
    }

    private fun apkFile(context: Context, path: String): File? = try {
        val dir = File(context.cacheDir, UPDATE_DIR).canonicalFile
        val file = File(path).canonicalFile
        file.takeIf { it.path.startsWith(dir.path + File.separator) && it.isFile && it.name.endsWith(".apk", ignoreCase = true) }
    } catch (e: IOException) {
        Log.w(TAG, "update APK path not resolvable: $path", e)
        null
    }
}
