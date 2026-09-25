package app.smartexplorer.android.core

import android.app.Application
import android.content.Context
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import android.os.Build
import android.os.Environment
import android.os.SystemClock
import android.provider.Settings
import app.smartexplorer.android.BuildConfig
import app.smartexplorer.android.system.Storage
import java.io.File
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/** Builds the `init` configuration of api.md §1. */
internal object InitConfig {
    /** Cache subfolders the core and the FileProvider (`res/xml/file_paths.xml`) rely on. */
    private val CACHE_DIRS = listOf("open", "share", "update", "tmp")
    private const val OVERRIDES_FILE = "test-overrides.json"

    fun build(app: Application): JsonObject {
        val cacheDir = app.cacheDir
        CACHE_DIRS.forEach { File(cacheDir, it).mkdirs() }
        val volumes = Storage.volumes(app)
        val info = packageInfo(app)
        val config = buildJsonObject {
            put("filesDir", app.filesDir.absolutePath)
            put("cacheDir", cacheDir.absolutePath)
            put("noBackupDir", app.noBackupFilesDir.absolutePath)
            put("appVersion", info.versionName ?: BuildConfig.VERSION_NAME)
            put("versionCode", info.longVersionCode)
            put("deviceName", deviceName(app))
            put("homeDir", volumes.firstOrNull { it.primary }?.path ?: defaultHome())
            put("bootMarker", bootMarker(app))
            put("updateFeedUrl", JsonNull)
            put("startDaemon", true)
            put("volumes", Core.json.encodeToJsonElement(ListSerializer(VolumeInfo.serializer()), volumes))
        }
        return if (BuildConfig.DEBUG) withTestOverrides(config, File(app.filesDir, OVERRIDES_FILE)) else config
    }

    /**
     * Debug builds only: top-level keys of `filesDir/test-overrides.json` replace the defaults
     * (e.g. `updateFeedUrl`, `startDaemon`). A malformed file fails the start loudly.
     */
    private fun withTestOverrides(config: JsonObject, file: File): JsonObject {
        if (!file.isFile) return config
        val overrides = Core.json.parseToJsonElement(file.readText()) as? JsonObject
            ?: throw CoreException("invalid", "$OVERRIDES_FILE ist kein JSON-Objekt")
        return JsonObject(config + overrides)
    }

    private fun packageInfo(context: Context): PackageInfo {
        val pm = context.packageManager
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            pm.getPackageInfo(context.packageName, PackageManager.PackageInfoFlags.of(0))
        } else {
            @Suppress("DEPRECATION")
            pm.getPackageInfo(context.packageName, 0)
        }
    }

    private fun deviceName(context: Context): String =
        Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME)
            ?.takeIf { it.isNotBlank() }
            ?: Build.MODEL

    /**
     * Marker that changes exactly once per device boot, for "on start" jobs.
     * `Settings.Global.BOOT_COUNT` (API 24+); the boot wall-clock minute is only a fallback.
     */
    private fun bootMarker(context: Context): String {
        val count = Settings.Global.getInt(context.contentResolver, Settings.Global.BOOT_COUNT, -1)
        if (count >= 0) return count.toString()
        val bootMs = System.currentTimeMillis() - SystemClock.elapsedRealtime()
        return "t" + bootMs / 60_000
    }

    @Suppress("DEPRECATION")
    private fun defaultHome(): String = Environment.getExternalStorageDirectory().absolutePath
}
