package app.smartexplorer.android.launcher

import android.app.ActivityManager
import android.app.role.RoleManager
import android.appwidget.AppWidgetHost
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Process
import android.util.Log
import java.io.File
import java.io.IOException

/**
 * Complete class names of the launcher module's components, as strings so the main process never
 * loads a launcher class (U35-2). `setComponentEnabledSetting` on a class that does not exist
 * throws, so every package lists only what it creates itself: U38 appended `PinRequestActivity` (DSK-M11) and
 * `BadgeListenerService` (DSK-M12) to [launcher]; U40 appends `LauncherBridgeService` to [launcher] and
 * `DevicesWidgetProvider` and `DevicesWidgetService` to [widget].
 */
internal object LauncherParts {
    val launcher: List<String> = listOf(
        "app.smartexplorer.android.launcher.LauncherActivity",
        "app.smartexplorer.android.launcher.pin.PinRequestActivity",
    )
    val widget: List<String> = emptyList()
}

sealed interface EnableResult {
    data object Done : EnableResult

    /** [message] is the complete text for the card („Aktivieren fehlgeschlagen: …“). */
    data class Failed(val message: String) : EnableResult
}

/** [error] is set when a step failed; the remaining steps still ran. */
data class DisableResult(val home: HomeResolution, val error: String? = null)

/** [deleteError] is the card's message when the files could not be deleted (U35-10). */
data class RemoveResult(val home: HomeResolution, val deleteError: String? = null, val error: String? = null)

/**
 * Main-process switch of the launcher module (DSK-M9, U35-5 to U35-10): enable, disable, remove.
 * The component state of `LauncherActivity` is the truth for „enabled“; `state.json` holds the
 * rest ([LauncherState]). Nothing here touches the `:launcher` process except killing it.
 */
object LauncherSwitch {
    private const val TAG = "LauncherSwitch"

    /** Host id of the launcher's `AppWidgetHost` (U35-3); U38's `LauncherWidgetHost` uses it unchanged. */
    const val WIDGET_HOST_ID = 0x5E57

    /** Suffix of the launcher process name (`<package>:launcher`, U35-11). */
    const val PROCESS_SUFFIX = ":launcher"

    private const val LAUNCHER_CLASS = "app.smartexplorer.android.launcher.LauncherActivity"

    /** True when `LauncherActivity` is switched on. */
    fun isComponentEnabled(context: Context): Boolean = context.packageManager.getComponentEnabledSetting(
        ComponentName(context.packageName, LAUNCHER_CLASS),
    ) == PackageManager.COMPONENT_ENABLED_STATE_ENABLED

    /** True when [LauncherParts.widget] is not empty and every component in it is on. */
    fun isWidgetPartEnabled(context: Context): Boolean = LauncherParts.widget.isNotEmpty() && LauncherParts.widget.all {
        context.packageManager.getComponentEnabledSetting(ComponentName(context.packageName, it)) ==
            PackageManager.COMPONENT_ENABLED_STATE_ENABLED
    }

    /**
     * Makes `state.json` agree with the component state (U35-5): after a crash between two steps the
     * component wins. Writes only on a difference, so a device that never used the launcher gets no file.
     */
    fun reconcile(context: Context): LauncherStateData {
        val app = context.applicationContext
        val stored = LauncherState.read(app)
        val launcher = isComponentEnabled(app)
        val widget = isWidgetPartEnabled(app)
        if (stored.enabled == launcher && stored.parts.launcher == launcher && stored.parts.widget == widget) return stored
        val fixed = stored.copy(
            state = if (launcher) LauncherStateData.STATE_ENABLED else LauncherStateData.STATE_DISABLED,
            parts = Parts(launcher, widget),
            changedAt = System.currentTimeMillis(),
        )
        try {
            LauncherState.write(app, fixed)
        } catch (e: IOException) {
            Log.w(TAG, "state.json not written", e)
        }
        return fixed
    }

    /** What the system currently answers for the HOME intent (U35-8). */
    fun resolveHome(context: Context): HomeResolution {
        val pm = context.packageManager
        val intent = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME)
        val info = pm.resolveActivity(intent, PackageManager.MATCH_DEFAULT_ONLY)?.activityInfo
        return HomeRules.classify(info?.packageName, info?.name, context.packageName) {
            info?.loadLabel(pm)?.toString().orEmpty().ifEmpty { info?.packageName.orEmpty() }
        }
    }

    /** `Other` as `<package>/<class>`, else `null`: what [enable] remembers (U35-6 step 1). */
    internal fun previousHomeOf(home: HomeResolution): String? = (home as? HomeResolution.Other)?.componentName

    /**
     * Enables the launcher components (U35-6): remember the previous home, write `state.json`, switch
     * the components on. The role request ([requestHomeRole]) follows in the card and changes nothing here.
     * A failure undoes the components and the file.
     */
    fun enable(context: Context): EnableResult {
        val app = context.applicationContext
        val current = LauncherState.read(app)
        val previous = previousHomeOf(resolveHome(app))
        try {
            LauncherState.write(
                app,
                LauncherStateData(
                    state = LauncherStateData.STATE_ENABLED,
                    parts = Parts(launcher = true, widget = current.parts.widget),
                    previousHome = previous,
                    changedAt = System.currentTimeMillis(),
                    noticeDismissed = false,
                ),
            )
        } catch (e: IOException) {
            return EnableResult.Failed("Aktivieren fehlgeschlagen: ${e.message ?: e.javaClass.simpleName}")
        }
        val switched = ArrayList<String>()
        try {
            for (name in LauncherParts.launcher) {
                setComponent(app, name, PackageManager.COMPONENT_ENABLED_STATE_ENABLED)
                switched += name
            }
        } catch (e: RuntimeException) {
            for (name in switched) runCatching { setComponent(app, name, PackageManager.COMPONENT_ENABLED_STATE_DISABLED) }
            runCatching { LauncherState.write(app, current.copy(state = LauncherStateData.STATE_DISABLED, parts = Parts(false, current.parts.widget))) }
            return EnableResult.Failed("Aktivieren fehlgeschlagen: ${e.message ?: e.javaClass.simpleName}")
        }
        return EnableResult.Done
    }

    /** The system dialog „Als Standard festlegen“, or `null` when the device has no role request (U35-7). */
    fun requestHomeRole(context: Context): Intent? {
        val roles = context.getSystemService(RoleManager::class.java) ?: return null
        return if (roles.isRoleAvailable(RoleManager.ROLE_HOME)) roles.createRequestRoleIntent(RoleManager.ROLE_HOME) else null
    }

    fun isRoleAvailable(context: Context): Boolean =
        context.getSystemService(RoleManager::class.java)?.isRoleAvailable(RoleManager.ROLE_HOME) == true

    fun isRoleHeld(context: Context): Boolean =
        context.getSystemService(RoleManager::class.java)?.isRoleHeld(RoleManager.ROLE_HOME) == true

    /**
     * Switches the widget part „Geräte-Widget für fremde Launcher“ (U35-13); with an empty [LauncherParts.widget] only the file changes.
     * Returns the new state.
     */
    fun setWidgetPart(context: Context, on: Boolean): LauncherStateData {
        val app = context.applicationContext
        val target = if (on) PackageManager.COMPONENT_ENABLED_STATE_ENABLED else PackageManager.COMPONENT_ENABLED_STATE_DISABLED
        LauncherParts.widget.forEach { setComponent(app, it, target) }
        val stored = LauncherState.read(app)
        val next = stored.copy(
            parts = stored.parts.copy(widget = on && LauncherParts.widget.isNotEmpty()),
            changedAt = System.currentTimeMillis(),
        )
        LauncherState.write(app, next)
        return next
    }

    /**
     * Deactivates (U35-9, C3.5): components off, host released, `:launcher` killed, then the file. The
     * layout stays. [DisableResult.home] feeds the card's statement ([HomeRules.notice]).
     */
    fun disable(context: Context): DisableResult {
        val app = context.applicationContext
        var error: String? = null
        for (name in LauncherParts.launcher + LauncherParts.widget) {
            try {
                setComponent(app, name, PackageManager.COMPONENT_ENABLED_STATE_DISABLED)
            } catch (e: RuntimeException) {
                error = error ?: "${e.message ?: e.javaClass.simpleName}"
                Log.w(TAG, "component not switched off: $name", e)
            }
        }
        releaseWidgetHost(app)
        killLauncherProcess(app)
        val stored = LauncherState.read(app)
        try {
            LauncherState.write(
                app,
                stored.copy(
                    state = LauncherStateData.STATE_DISABLED,
                    parts = Parts(launcher = false, widget = false),
                    changedAt = System.currentTimeMillis(),
                    noticeDismissed = false,
                ),
            )
        } catch (e: IOException) {
            error = error ?: "${e.message ?: e.javaClass.simpleName}"
        }
        return DisableResult(resolveHome(app), error)
    }

    /**
     * Removes the launcher (U35-10): [disable], components back to the manifest state, then the
     * files. Killing the process comes first; otherwise it would write `layout.json` again.
     */
    fun remove(context: Context): RemoveResult {
        val app = context.applicationContext
        val disabled = disable(app)
        var error = disabled.error
        for (name in LauncherParts.launcher + LauncherParts.widget) {
            try {
                setComponent(app, name, PackageManager.COMPONENT_ENABLED_STATE_DEFAULT)
            } catch (e: RuntimeException) {
                error = error ?: "${e.message ?: e.javaClass.simpleName}"
            }
        }
        val dir = LauncherState.dir(app)
        var deleteError: String? = null
        if (dir.exists() && !dir.deleteRecursively()) {
            deleteError = "Dateien des Launchers konnten nicht gelöscht werden: ${dir.path}"
            // state.json stays „disabled“ even if the failed deletion removed it first.
            runCatching { LauncherState.write(app, LauncherStateData.DISABLED.copy(changedAt = System.currentTimeMillis())) }
        }
        for (extra in listOf(File(app.cacheDir, "launcher-icons"), File(app.filesDir, "widget"))) {
            if (extra.exists() && !extra.deleteRecursively() && deleteError == null) {
                deleteError = "Dateien des Launchers konnten nicht gelöscht werden: ${extra.path}"
            }
        }
        return RemoveResult(disabled.home, deleteError, error)
    }

    /** Remembers that the person saw the „Wähle deinen Startbildschirm“ notice (U35-9, „Verstanden“). */
    fun dismissNotice(context: Context) {
        val app = context.applicationContext
        try {
            LauncherState.write(app, LauncherState.read(app).copy(noticeDismissed = true))
        } catch (e: IOException) {
            Log.w(TAG, "state.json not written", e)
        }
    }

    /** Name of the launcher stored as `<package>/<class>`, or `null` when it is no longer installed (U35-9). */
    fun previousHomeLabel(context: Context, previousHome: String): String? {
        val component = ComponentName.unflattenFromString(previousHome) ?: return null
        val pm = context.packageManager
        return try {
            pm.getActivityInfo(component, 0).loadLabel(pm).toString()
        } catch (_: PackageManager.NameNotFoundException) {
            null
        }
    }

    private fun setComponent(context: Context, className: String, state: Int) {
        context.packageManager.setComponentEnabledSetting(
            ComponentName(context.packageName, className),
            state,
            PackageManager.DONT_KILL_APP,
        )
    }

    /** `deleteHost` frees every widget id; the „always allow“ approval of the bind stays (DN18). */
    private fun releaseWidgetHost(context: Context) {
        try {
            AppWidgetHost(context, WIDGET_HOST_ID).deleteHost()
        } catch (e: RuntimeException) {
            Log.w(TAG, "deleteHost failed", e)
        }
    }

    private fun killLauncherProcess(context: Context) {
        val name = context.packageName + PROCESS_SUFFIX
        val processes = context.getSystemService(ActivityManager::class.java)?.runningAppProcesses ?: return
        processes.filter { it.processName == name }.forEach { Process.killProcess(it.pid) }
    }
}
