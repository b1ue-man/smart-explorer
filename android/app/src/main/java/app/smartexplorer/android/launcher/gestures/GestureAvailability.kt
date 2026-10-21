package app.smartexplorer.android.launcher.gestures

import android.content.Context
import android.provider.Settings

/** Whether the two gestures can work: the accessibility service of the app is switched on (U40-18). */
enum class Availability { On, Off }

/**
 * Reads, from the launcher process, whether the person switched on the app's accessibility service
 * (`ScreenControlService`, SCR-M8). Both gestures run through that service (`GLOBAL_ACTION` of the bridge); without it
 * they do nothing but say why (U40-18, E7). The class name is a string, so the launcher process loads no class of the main
 * process, and the check reads only the system setting, not the service object (which lives in the main process).
 */
object GestureAvailability {
    const val SERVICE_CLASS = "app.smartexplorer.android.screen.ScreenControlService"

    fun check(context: Context): Availability {
        val list = Settings.Secure.getString(context.contentResolver, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES)
        return if (listed(list, context.packageName, SERVICE_CLASS)) Availability.On else Availability.Off
    }

    /**
     * True when [setting] (service names separated by `:`) names `<[packageName]>/<[serviceClass]>`, ignoring case. The system
     * writes a class of the app's own package either in full or in the short form `<package>/.<rest>`; both count.
     */
    fun listed(setting: String?, packageName: String, serviceClass: String): Boolean {
        if (setting.isNullOrEmpty()) return false
        val wanted = "$packageName/$serviceClass"
        return setting.split(':').any { entry ->
            val name = entry.trim()
            name.equals(wanted, ignoreCase = true) || name.equals(shortForm(packageName, serviceClass), ignoreCase = true)
        }
    }

    private fun shortForm(packageName: String, serviceClass: String): String =
        if (serviceClass.startsWith("$packageName.")) "$packageName/${serviceClass.removePrefix(packageName)}" else "$packageName/$serviceClass"
}
