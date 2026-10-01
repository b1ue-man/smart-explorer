package app.smartexplorer.android.system

import android.Manifest
import android.app.Activity
import android.app.AppOpsManager
import android.app.usage.StorageStatsManager
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.Process
import android.os.StatFs
import android.os.UserHandle
import android.os.storage.StorageManager
import android.provider.Settings
import android.util.Log
import app.smartexplorer.android.api.AnalyzeApp
import app.smartexplorer.android.api.AnalyzePlatform
import java.io.File
import java.io.IOException
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext

/**
 * Platform storage figures for the storage analysis (spec B2/B3/B6, android-storage-scan.md §1):
 * used space of the volume that holds the analysed root (`StatFs`, no permission), the summed
 * `Android/data` space of all apps on the primary volume (`ExternalStorageStats.getAppBytes()`) and,
 * for the whole primary volume, every installed app's storage (`PackageManager` with
 * `QUERY_ALL_PACKAGES` + `StorageStatsManager.queryStatsForPackage`). The last two need "Zugriff auf
 * Nutzungsdaten" = app op `GET_USAGE_STATS`. Android 11+ locks other apps' `Android/data|obb`
 * folders for every app, so these figures are the only way to size them.
 */
object StorageStatsAccess {
    private const val TAG = "StorageStatsAccess"
    private val APP_DATA = listOf("Android", "data")

    /**
     * App queries in flight at once. Each one holds a binder thread of `system_server` (a pool
     * shared by every app) and of `installd` until its quota lookup returns; four overlap the round
     * trips without taking those shared pools from the rest of the system.
     */
    private const val APP_QUERIES = 4

    /** Figures for one analysis root; `null` fields are unknown. */
    class Figures(
        val volumeUsedBytes: Long?,
        val otherAppsBytes: Long?,
        /** The analysed tree contains `Android/data` of the primary volume (other apps' data counts there). */
        val appDataInTree: Boolean,
        /** Installed apps with code or data on the volume (only the whole primary volume); `null` = no list. */
        val apps: List<AnalyzeApp>? = null,
    ) {
        /** `analyze.start.platform` of these figures. */
        val platform: AnalyzePlatform
            get() = AnalyzePlatform(volumeUsedBytes, otherAppsBytes, apps)

        /** The figures hold Android's sizes of other apps (made with usage access). */
        val includesApps: Boolean
            get() = otherAppsBytes != null || apps != null
    }

    /**
     * Figures for [location], or `null` when it is no local path on a mounted volume (remote places,
     * Share devices, paths outside the storage volumes). Runs on the IO dispatcher (the stats calls
     * "may take several seconds"), once per analysis; the app list is queried alongside the volume
     * figures.
     */
    suspend fun figuresFor(context: Context, location: String): Figures? = withContext(Dispatchers.IO) {
        if (!isLocalPath(location)) return@withContext null
        val manager = context.getSystemService(StorageManager::class.java) ?: return@withContext null
        val root = File(location.trimStart())
        // Canonical containment, so aliases such as /sdcard resolve to their volume.
        val volume = manager.getStorageVolume(root) ?: return@withContext null
        if (volume.state != Environment.MEDIA_MOUNTED && volume.state != Environment.MEDIA_MOUNTED_READ_ONLY) {
            return@withContext null
        }
        val volumeDir = volume.directory ?: return@withContext null
        val below = segmentsBelow(volumeDir, root)
        val appDataInTree = volume.isPrimary && below != null && below.size <= APP_DATA.size &&
            below.indices.all { below[it].equals(APP_DATA[it], ignoreCase = true) }
        val uuid = if (appDataInTree && hasUsageAccess(context)) uuidOf(manager, volumeDir) else null
        coroutineScope {
            val apps = async { if (uuid != null && below?.isEmpty() == true) installedApps(context, uuid) else null }
            val other = uuid?.let { appDataBytes(context, it) }
            Figures(usedBytes(volumeDir), other, appDataInTree, apps.await())
        }
    }

    /** A local file path (api.md §2: no `scheme://`; the core ignores leading blanks), not a remote or app-internal place. */
    fun isLocalPath(location: String): Boolean {
        val path = location.trimStart()
        return path.startsWith("/") && !path.contains("://")
    }

    /**
     * `true` while usage access is granted (mirrors `StorageStatsService.checkStatsPermission`).
     * `checkOpNoThrow(String, int, String)` exists since API 19, is un-deprecated in API 36 (where the
     * `unsafeCheckOp*` family became deprecated) and checks the same mode without noting the op.
     */
    fun hasUsageAccess(context: Context): Boolean {
        val ops = context.getSystemService(AppOpsManager::class.java) ?: return false
        return when (ops.checkOpNoThrow(AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), context.packageName)) {
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
        launch(context, Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS, packageUri(context.packageName))) ||
            launch(context, Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS))

    /**
     * App info page of [packageName] (default: this app, whose ⋮ menu holds "Eingeschränkte
     * Einstellungen zulassen"; for other apps it offers "Cache leeren" and "Speicher löschen").
     */
    fun openAppDetails(context: Context, packageName: String = context.packageName): Boolean =
        launch(context, Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, packageUri(packageName)))

    /** Names of [root] below [volumeDir] on canonical paths; `null` when it does not lie on it. */
    private fun segmentsBelow(volumeDir: File, root: File): List<String>? {
        val base = canonical(volumeDir) ?: return null
        val target = canonical(root) ?: return null
        if (target != base && !target.startsWith(base.trimEnd('/') + "/")) return null
        return target.removePrefix(base).split('/').filter { it.isNotEmpty() }
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

    private fun uuidOf(manager: StorageManager, volumeDir: File): UUID? = try {
        manager.getUuidForPath(volumeDir)
    } catch (e: IOException) {
        Log.w(TAG, "volume without storage UUID: ${e.message}")
        null
    }

    /**
     * `Android/data` (incl. cache) of all apps on the volume; `null` when the platform cannot tell
     * (the figure is optional, the analysis runs without it).
     */
    private fun appDataBytes(context: Context, uuid: UUID): Long? {
        val stats = context.getSystemService(StorageStatsManager::class.java) ?: return null
        return try {
            stats.queryExternalStatsForUser(uuid, Process.myUserHandle()).appBytes.takeIf { it >= 0 }
        } catch (e: IOException) {
            Log.w(TAG, "external storage stats unavailable: ${e.message}")
            null
        } catch (e: RuntimeException) {
            // SecurityException (access revoked meanwhile) and service-side failures of this optional figure.
            Log.w(TAG, "external storage stats failed: ${e.javaClass.simpleName}: ${e.message}")
            null
        }
    }

    /**
     * Every package of this user (also uninstalled ones whose data Android kept) with its code and
     * data on the volume [uuid]; `null` when Android cannot list them or usage access went away
     * meanwhile (no list rather than a partial one), packages without bytes there are left out.
     */
    private suspend fun installedApps(context: Context, uuid: UUID): List<AnalyzeApp>? {
        val packages = context.packageManager
        val stats = context.getSystemService(StorageStatsManager::class.java) ?: return null
        val infos = try {
            installedApplications(packages)
        } catch (e: RuntimeException) {
            Log.w(TAG, "installed apps unavailable: ${e.javaClass.simpleName}: ${e.message}")
            return null
        }
        val user = Process.myUserHandle()
        val permits = Semaphore(APP_QUERIES)
        val apps = try {
            coroutineScope {
                infos.map { info -> async { permits.withPermit { appStorage(packages, stats, uuid, user, info) } } }.awaitAll()
            }
        } catch (e: SecurityException) {
            Log.w(TAG, "app storage stats denied: ${e.message}")
            return null
        }
        // The own app always has bytes: an empty list means Android measured nothing.
        return apps.filterNotNull().takeIf { it.isNotEmpty() }
    }

    private fun installedApplications(packages: PackageManager): List<ApplicationInfo> =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            packages.getInstalledApplications(
                PackageManager.ApplicationInfoFlags.of(PackageManager.MATCH_UNINSTALLED_PACKAGES.toLong()),
            )
        } else {
            @Suppress("DEPRECATION")
            packages.getInstalledApplications(PackageManager.MATCH_UNINSTALLED_PACKAGES)
        }

    /**
     * One package's figures on the volume [uuid]; `null` without bytes there or when Android cannot
     * tell (package removed meanwhile, measurement failed). A revoked usage access ends the list.
     */
    private fun appStorage(
        packages: PackageManager,
        stats: StorageStatsManager,
        uuid: UUID,
        user: UserHandle,
        info: ApplicationInfo,
    ): AnalyzeApp? {
        val storage = try {
            stats.queryStatsForPackage(uuid, info.packageName, user)
        } catch (e: PackageManager.NameNotFoundException) {
            return null
        } catch (e: IOException) {
            Log.w(TAG, "storage stats of ${info.packageName} unavailable: ${e.message}")
            return null
        } catch (e: SecurityException) {
            throw e
        } catch (e: RuntimeException) {
            Log.w(TAG, "storage stats of ${info.packageName} failed: ${e.javaClass.simpleName}: ${e.message}")
            return null
        }
        val appBytes = storage.appBytes.coerceAtLeast(0)
        val dataBytes = storage.dataBytes.coerceAtLeast(0)
        if (appBytes == 0L && dataBytes == 0L) return null
        return AnalyzeApp(
            packageName = info.packageName,
            label = labelOf(packages, info),
            appBytes = appBytes,
            dataBytes = dataBytes,
            cacheBytes = storage.cacheBytes.coerceIn(0, dataBytes),
        )
    }

    /** The app's name; packages Android only keeps data of are marked as such. */
    private fun labelOf(packages: PackageManager, info: ApplicationInfo): String {
        val loaded = try {
            packages.getApplicationLabel(info).toString().trim()
        } catch (e: RuntimeException) {
            ""
        }
        val label = loaded.ifEmpty { info.packageName }
        val installed = (info.flags and ApplicationInfo.FLAG_INSTALLED) != 0
        return if (installed) label else "$label (nicht installiert)"
    }

    private fun packageUri(packageName: String): Uri = Uri.parse("package:$packageName")

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
