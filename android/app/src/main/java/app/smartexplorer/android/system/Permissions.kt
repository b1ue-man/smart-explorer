package app.smartexplorer.android.system

import android.Manifest
import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.PowerManager
import android.provider.Settings
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat

/** Status checks and settings shortcuts for the special permissions the app relies on. */
object Permissions {
    /** "Zugriff auf alle Dateien" (MANAGE_EXTERNAL_STORAGE); there is no runtime dialog for it. */
    fun hasAllFilesAccess(): Boolean = Environment.isExternalStorageManager()

    /** Opens the per-app all-files-access page, falling back to the general list. */
    fun openAllFilesAccessSettings(context: Context): Boolean =
        launch(context, Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION, packageUri(context))) ||
            launch(context, Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION))

    /** `true` when POST_NOTIFICATIONS is a runtime permission on this device (API 33+). */
    fun notificationsNeedRuntimePermission(): Boolean = Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU

    fun canPostNotifications(context: Context): Boolean {
        if (notificationsNeedRuntimePermission() &&
            ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            return false
        }
        return NotificationManagerCompat.from(context).areNotificationsEnabled()
    }

    /** App notification settings (used when the runtime dialog is no longer offered). */
    fun openNotificationSettings(context: Context): Boolean =
        launch(
            context,
            Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName),
        )

    /** "Unbekannte Apps installieren" for self-updates. */
    fun canInstallPackages(context: Context): Boolean = context.packageManager.canRequestPackageInstalls()

    fun openInstallPackagesSettings(context: Context): Boolean =
        launch(context, Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, packageUri(context)))

    fun isIgnoringBatteryOptimizations(context: Context): Boolean =
        context.getSystemService(PowerManager::class.java)?.isIgnoringBatteryOptimizations(context.packageName) == true

    /** Shows the system dialog that exempts the app from battery optimization. */
    fun requestIgnoreBatteryOptimizations(context: Context): Boolean =
        launch(context, Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, packageUri(context)))

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
