package app.smartexplorer.android.launcher.model

import android.content.Context
import android.content.Intent
import android.content.pm.LauncherApps
import android.content.pm.PackageManager
import android.os.Process
import android.os.UserManager
import android.provider.MediaStore
import android.provider.Settings
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/**
 * The first arrangement when `layout.json` is missing or was set aside (U35-20). New installs do not
 * appear on the home screen on their own (P24), but an empty one would be unusable: the dock gets up
 * to four basic apps, page 1 this app and the common ones, each only when the device has it.
 */
object FirstRun {
    private const val OWN_ACTIVITY = "app.smartexplorer.android.MainActivity"
    private const val FIRST_ICON_DP = 56

    fun build(context: Context): HomeLayout {
        val launcherApps = context.getSystemService(LauncherApps::class.java)
        val user = Process.myUserHandle()
        val serial = context.getSystemService(UserManager::class.java).getSerialNumberForUser(user)
        val metrics = context.resources.displayMetrics
        // The page area without insets, as far as known here; the display places the cells again.
        val widthDp = metrics.widthPixels / metrics.density
        val heightDp = metrics.heightPixels / metrics.density - GridGeometry.dockHeightDp(FIRST_ICON_DP) - GridGeometry.INDICATOR_DP - 96f
        val geometry = GridGeometry.geometry(widthDp, heightDp, FIRST_ICON_DP)
        val taken = HashSet<String>()

        fun appItem(component: String, cell: Cell): HomeItem {
            val id = LayoutJson.newId(taken)
            taken += id
            return HomeItem(
                id = id,
                kind = Kinds.APP,
                cell = cell,
                attrs = JsonObject(mapOf("component" to JsonPrimitive(component), "user" to JsonPrimitive(serial))),
            )
        }

        fun launcherComponent(intent: Intent): String? {
            val pm = context.packageManager
            @Suppress("DEPRECATION")
            val info = pm.queryIntentActivities(intent, PackageManager.MATCH_DEFAULT_ONLY).firstOrNull()?.activityInfo ?: return null
            // The item must start through LauncherApps, so it needs a launcher activity of the same app.
            val activities = launcherApps.getActivityList(info.packageName, user)
            val chosen = activities.firstOrNull { it.name == info.name } ?: activities.firstOrNull() ?: return null
            return chosen.componentName.flattenToString()
        }

        val dockComponents = listOf(
            Intent(Intent.ACTION_DIAL),
            Intent.makeMainSelectorActivity(Intent.ACTION_MAIN, Intent.CATEGORY_APP_MESSAGING),
            Intent.makeMainSelectorActivity(Intent.ACTION_MAIN, Intent.CATEGORY_APP_BROWSER),
            Intent(MediaStore.INTENT_ACTION_STILL_IMAGE_CAMERA),
        ).mapNotNull { launcherComponent(it) }.distinct()
        val dock = dockComponents.mapIndexed { index, component -> appItem(component, Cell(index, 0)) }

        val own = "${context.packageName}/$OWN_ACTIVITY"
        val pageComponents = (
            listOf(own) + listOf(
                Intent(Settings.ACTION_SETTINGS),
                Intent.makeMainSelectorActivity(Intent.ACTION_MAIN, Intent.CATEGORY_APP_GALLERY),
                Intent.makeMainSelectorActivity(Intent.ACTION_MAIN, Intent.CATEGORY_APP_MAPS),
                Intent.makeMainSelectorActivity(Intent.ACTION_MAIN, Intent.CATEGORY_APP_EMAIL),
                Intent.makeMainSelectorActivity(Intent.ACTION_MAIN, Intent.CATEGORY_APP_CALENDAR),
                Intent.makeMainSelectorActivity(Intent.ACTION_MAIN, Intent.CATEGORY_APP_MUSIC),
                Intent.makeMainSelectorActivity(Intent.ACTION_MAIN, Intent.CATEGORY_APP_CONTACTS),
            ).mapNotNull { launcherComponent(it) }
            ).distinct().filter { it !in dockComponents }
        val page = pageComponents.mapIndexed { index, component ->
            appItem(component, Cell(index % geometry.columns, index / geometry.columns))
        }
        return HomeLayout(
            grid = GridSpec(geometry.columns, geometry.rows, FIRST_ICON_DP),
            pages = listOf(page),
            dock = dock,
            extra = JsonObject(mapOf("seSeeded" to JsonPrimitive(false))),
        )
    }
}
