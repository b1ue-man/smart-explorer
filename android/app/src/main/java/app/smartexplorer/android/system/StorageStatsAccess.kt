package app.smartexplorer.android.system

import android.Manifest
import android.app.Activity
import android.app.AppOpsManager
import android.app.usage.StorageStatsManager
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.Process
import android.os.StatFs
import android.os.storage.StorageManager
import android.provider.Settings
import android.util.Log
import androidx.annotation.WorkerThread
import java.io.File
import java.io.IOException

/**
 * Platform storage figures for the storage analysis (spec B2/B3, android-storage-scan.md §1):
 * used space of the volume that holds the analysed root (`StatFs`, no permission) and the summed
 * `Android/data` space of all apps on the primary volume (`ExternalStorageStats.getAppBytes()`,
 * needs "Zugriff auf Nutzungsdaten" = app op `GET_USAGE_STATS`). Android 11+ locks other apps'
 * `Android/data|obb` folders for every app, so these figures are the only way to size them.
 */
object StorageStatsAccess {
    private const val TAG = "StorageStatsAccess"
    private val APP_DATA = listOf("Android", "data")

    /** Figures for one analysis root; `null` fields are unknown. */
    class Figures(
        val volumeUsedBytes: Long?,
        val otherAppsBytes: Long?,
        /** The analysed tree contains `Android/data` of the primary volume (other apps' data counts there). */
        val appDataInTree: Boolean,
    )

    /**
     * Figures for [location], or `null` when it is no local path on a mounted volume (remote places,
     * Share devices, paths outside the storage volumes). Blocking: `queryExternalStatsForUser` "may
     * take several seconds", so call it off the main thread, once per analysis.
     */
    @WorkerThread
    fun figuresFor(context: Context, location: String): Figures? {
        if (!isLocalPath(location)) return null
        val path = location.trimStart()
        val manager = context.getSystemService(StorageManager::class.java) ?: return null
        val root = File(path)
        // Canonical containment, so aliases such as /sdcard resolve to their volume.
        val volume = manager.getStorageVolume(root) ?: return null
        if (volume.state != Environment.MEDIA_MOUNTED && volume.state != Environment.MEDIA_MOUNTED_READ_ONLY) return null
        val volumeDir = volume.directory ?: return null
        val appDataInTree = volume.isPrimary && containsAppData(volumeDir, root)
        return Figures(
            volumeUsedBytes = usedBytes(volumeDir),
            otherAppsBytes = if (appDataInTree) appDataBytes(context, manager, volumeDir) else null,
            appDataInTree = appDataInTree,
        )
    }

    /** A local file path (api.md §2: no `scheme://`; the core ignores leading blanks), not a remote or app-internal place. */
    fun isLocalPath(location: String): Boolean {
        val path = location.trimStart()
        return path.startsWith("/") && !path.contains("://")
    }

    /** `true` while usage access is granted (mirrors `StorageStatsService.checkStatsPermission`). */
    fun hasUsageAccess(context: Context): Boolean {
        val ops = context.getSystemService(AppOpsManager::class.java) ?: return false
        return when (ops.unsafeCheckOpNoThrow(AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), context.packageName)) {
            AppOpsManager.MODE_ALLOWED -> true
            AppOpsManager.MODE_DEFAULT ->
                context.checkSelfPermission(Manifest.permission.PACKAGE_USAGE_STATS) == PackageManager.PERMISSION_GRANTED
            else -> false
        }
    }

    /**
     * Android 15+ guards usage access of sideloaded apps ("Restricted settings"): the switch stays
     * locked until App-Info → ⋮ → "Eingeschränkte Einstellungen zulassen".
     */
    val restrictedSettingsPossible: Boolean
        get() = Build.VERSION.SDK_INT >= Build.VERSION_CODES.VANILLA_ICE_CREAM

    /** Usage access page of this app (undocumented `package:` form), else the general list. */
    fun openUsageAccessSettings(context: Context): Boolean =
        launch(context, Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS, packageUri(context))) ||
            launch(context, Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS))

    /** App info page (its ⋮ menu holds "Eingeschränkte Einstellungen zulassen"). */
    fun openAppDetails(context: Context): Boolean =
        launch(context, Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, packageUri(context)))

    /** Root is the volume itself, its `Android` or its `Android/data` folder (FUSE names are case-insensitive). */
    private fun containsAppData(volumeDir: File, root: File): Boolean {
        val base = canonical(volumeDir) ?: return false
        val target = canonical(root) ?: return false
        if (target != base && !target.startsWith(base.trimEnd('/') + "/")) return false
        val segments = target.removePrefix(base).split('/').filter { it.isNotEmpty() }
        return segments.size <= APP_DATA.size && segments.indices.all { segments[it].equals(APP_DATA[it], ignoreCase = true) }
    }

    private fun canonical(file: File): String? = try {
        file.canonicalPath
    } catch (e: IOException) {
        null
    }

    /** `f_blocks − f_bfree` like `df` (reserved blocks count as free, not as used). */
    private fun usedBytes(volumeDir: File): Long? = try {
        val stat = StatFs(volumeDir.path)
        (stat.totalBytes - stat.freeBytes).takeIf { it >= 0 }
    } catch (e: IllegalArgumentException) {
        null
    }

    /**
     * `Android/data` (incl. cache) of all apps on the volume; `null` without usage access or when
     * the platform cannot tell (the figure is optional, the analysis runs without it).
     */
    private fun appDataBytes(context: Context, manager: StorageManager, volumeDir: File): Long? {
        if (!hasUsageAccess(context)) return null
        val stats = context.getSystemService(StorageStatsManager::class.java) ?: return null
        return try {
            stats.queryExternalStatsForUser(manager.getUuidForPath(volumeDir), Process.myUserHandle()).appBytes.takeIf { it >= 0 }
        } catch (e: IOException) {
            Log.w(TAG, "external storage stats unavailable: ${e.message}")
            null
        } catch (e: RuntimeException) {
            // SecurityException (access revoked meanwhile) and service-side failures of this optional figure.
            Log.w(TAG, "external storage stats failed: ${e.javaClass.simpleName}: ${e.message}")
            null
        }
    }

    private fun packageUri(context: Context): Uri = Uri.parse("package:${context.packageName}")

    /** Starts a settings activity; `false` when the device offers none for this intent. */
    private fun launch(context: Context, intent: Intent): Boolean {
        if (context !is Activity) intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        return try {
            context.startActivity(intent)
            true
        } catch (e: ActivityNotFoundException) {
            false
        }
    }
}
