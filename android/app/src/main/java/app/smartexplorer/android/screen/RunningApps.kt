package app.smartexplorer.android.screen

import android.app.AppOpsManager
import android.app.usage.UsageStatsManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Process
import app.smartexplorer.android.api.ScreenAppEntry

/**
 * „Was läuft“ of this phone as host (C5): the apps with a window from the accessibility service
 * (`flagRetrieveInteractiveWindows`), otherwise the apps used in the last [RECENT_MS] from the usage
 * statistics, if the user granted usage access. Icon references keep that package identity.
 */
internal object RunningApps {
    private const val RECENT_MS = 60 * 60 * 1000L
    private const val MAX_APPS = 32

    /** Whether the user granted usage access (`PACKAGE_USAGE_STATS`). */
    fun usageAccess(context: Context): Boolean {
        val ops = context.getSystemService(AppOpsManager::class.java) ?: return false
        val mode = ops.unsafeCheckOpNoThrow(AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), context.packageName)
        return mode == AppOpsManager.MODE_ALLOWED
    }

    fun list(context: Context): List<ScreenAppEntry> {
        val apps = current(context)
        AppIcons.retainPackages(apps.map { it.packageName }.toSet())
        return apps
    }

    private fun entry(context: Context, pkg: String, title: String) =
        ScreenAppEntry(pkg, label(context, pkg) ?: title, icon = AppIcons.reference(context, pkg))

    private fun current(context: Context): List<ScreenAppEntry> {
        ScreenControlService.current()?.let { service ->
            val windows = service.windowApps()
            if (windows.isNotEmpty()) return windows.take(MAX_APPS).map { (pkg, title) -> entry(context, pkg, title) }
        }
        if (!usageAccess(context)) return emptyList()
        val usage = context.getSystemService(UsageStatsManager::class.java) ?: return emptyList()
        val now = System.currentTimeMillis()
        return usage.queryUsageStats(UsageStatsManager.INTERVAL_DAILY, now - RECENT_MS, now)
            .filter { it.lastTimeUsed >= now - RECENT_MS && it.packageName != context.packageName }
            .sortedByDescending { it.lastTimeUsed }
            .map { it.packageName }
            .distinct()
            .take(MAX_APPS)
            .map { entry(context, it, it) }
    }

    private fun label(context: Context, pkg: String): String? = try {
        val info = context.packageManager.getApplicationInfo(pkg, 0)
        context.packageManager.getApplicationLabel(info).toString()
    } catch (e: PackageManager.NameNotFoundException) {
        null
    }
}
