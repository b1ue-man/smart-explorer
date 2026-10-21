package app.smartexplorer.android.launcher.shortcuts

import android.content.ComponentName
import android.content.Context
import android.content.pm.LauncherApps
import android.content.pm.ShortcutInfo
import android.graphics.drawable.Drawable
import android.os.UserHandle

/**
 * App shortcuts through `LauncherApps` (U35-26). Without the shortcut host permission (the launcher is not
 * the default home app) every function returns nothing and the menu has no „Verknüpfungen“ section.
 */
class Shortcuts(context: Context) {
    private val launcherApps = context.getSystemService(LauncherApps::class.java)

    fun hasHostPermission(): Boolean = try {
        launcherApps.hasShortcutHostPermission()
    } catch (_: SecurityException) {
        false
    }

    private fun query(user: UserHandle, flags: Int, configure: LauncherApps.ShortcutQuery.() -> Unit): List<ShortcutInfo> {
        if (!hasHostPermission()) return emptyList()
        return try {
            launcherApps.getShortcuts(LauncherApps.ShortcutQuery().setQueryFlags(flags).apply(configure), user).orEmpty()
        } catch (_: SecurityException) {
            emptyList()
        } catch (_: IllegalStateException) {
            // The profile is locked.
            emptyList()
        }
    }

    /** Dynamic and static shortcuts of one activity, in the order of their rank. */
    fun forActivity(component: ComponentName, user: UserHandle): List<ShortcutInfo> =
        query(user, LauncherApps.ShortcutQuery.FLAG_MATCH_DYNAMIC or LauncherApps.ShortcutQuery.FLAG_MATCH_MANIFEST) {
            setPackage(component.packageName)
            setActivity(component)
        }.sortedBy { it.rank }

    /** Shortcuts of a package for the search (dynamic, static and pinned). */
    fun forPackage(packageName: String, user: UserHandle): List<ShortcutInfo> = query(
        user,
        LauncherApps.ShortcutQuery.FLAG_MATCH_DYNAMIC or LauncherApps.ShortcutQuery.FLAG_MATCH_MANIFEST or LauncherApps.ShortcutQuery.FLAG_MATCH_PINNED,
    ) { setPackage(packageName) }.sortedBy { it.rank }

    /** The shortcuts of [packageName] that this launcher has pinned. */
    fun pinned(packageName: String, user: UserHandle): List<ShortcutInfo> =
        query(user, LauncherApps.ShortcutQuery.FLAG_MATCH_PINNED) { setPackage(packageName) }

    /** Replaces the pinned shortcuts of [packageName] by [ids]; the caller passes the union with those it keeps. */
    fun pin(packageName: String, ids: List<String>, user: UserHandle) {
        if (!hasHostPermission()) return
        try {
            launcherApps.pinShortcuts(packageName, ids, user)
        } catch (_: SecurityException) {
            // Lost the host permission meanwhile; the item stays and its start reports the problem.
        }
    }

    fun start(shortcut: ShortcutInfo): Boolean = try {
        launcherApps.startShortcut(shortcut, null, null)
        true
    } catch (_: Exception) {
        false
    }

    /** Starts a pinned shortcut by its id (the home screen keeps only the id). */
    fun startById(packageName: String, shortcutId: String, user: UserHandle): Boolean = try {
        launcherApps.startShortcut(packageName, shortcutId, null, null, user)
        true
    } catch (_: Exception) {
        false
    }

    fun icon(shortcut: ShortcutInfo, densityDpi: Int): Drawable? = try {
        launcherApps.getShortcutIconDrawable(shortcut, densityDpi)
    } catch (_: SecurityException) {
        null
    }
}
