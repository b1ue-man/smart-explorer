package app.smartexplorer.android.screen

import android.app.KeyguardManager
import android.content.ComponentName
import android.content.Context
import android.content.pm.LauncherApps
import android.os.Build
import android.os.Process
import android.os.UserManager
import android.provider.Settings
import java.security.MessageDigest
import java.util.UUID
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
internal data class AppHostSession(
    @SerialName("user_id") val userId: String,
    @SerialName("session_id") val sessionId: String,
    val interactive: Boolean,
    val unlocked: Boolean,
)

@Serializable
internal data class HostCatalogApp(
    @SerialName("app_id") val appId: String,
    val name: String,
    val kind: String = "android_launcher",
    val icon: ScreenIconRef? = null,
    @SerialName("can_launch") val canLaunch: Boolean,
    @SerialName("unavailable_reason") val unavailableReason: String? = null,
)

@Serializable
internal data class AndroidInstalledApp(val app: HostCatalogApp, val fingerprint: String)

@Serializable
internal data class AndroidAppCatalog(
    val session: AppHostSession,
    val apps: List<AndroidInstalledApp>,
    val icons: List<ScreenIconReply> = emptyList(),
    val limits: List<String> = emptyList(),
)

internal data class ResolvedLauncherApp(val component: ComponentName, val entry: AndroidInstalledApp)
internal data class LocalAppCatalog(val wire: AndroidAppCatalog, val targets: Map<String, ResolvedLauncherApp>)

/** Only LauncherApps/PackageManager results create launch targets. App IDs never become paths. */
internal object AppCatalog {
    private const val MAX_APPS = 8192
    private const val MAX_ICON_PAYLOAD = 4 * 1024 * 1024
    private val processIncarnation = UUID.randomUUID().toString()

    fun hash(text: String): String = MessageDigest.getInstance("SHA-256")
        .digest(text.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it.toInt() and 255) }

    /** A process restart/user stop changes the incarnation; stale grants cannot cross it. */
    fun session(context: Context): AppHostSession {
        val users = requireNotNull(context.getSystemService(UserManager::class.java))
        val handle = Process.myUserHandle()
        val serial = users.getSerialNumberForUser(handle)
        check(serial >= 0) { "Android user serial is unavailable" }
        val boot = Settings.Global.getInt(context.contentResolver, Settings.Global.BOOT_COUNT, -1)
        val foreground = if (Build.VERSION.SDK_INT >= 31) users.isUserForeground else users.isUserRunning(handle)
        val unlocked = users.isUserUnlocked &&
            context.getSystemService(KeyguardManager::class.java)?.isKeyguardLocked == false
        return AppHostSession("android-user:$serial", "android-login:$boot:$serial:$processIncarnation", foreground, unlocked)
    }

    @Suppress("DEPRECATION")
    fun read(context: Context): LocalAppCatalog {
        val before = session(context)
        val launcher = requireNotNull(context.getSystemService(LauncherApps::class.java))
        val user = Process.myUserHandle()
        val serial = requireNotNull(context.getSystemService(UserManager::class.java)).getSerialNumberForUser(user)
        val pm = context.packageManager
        val activities = launcher.getActivityList(null, user)
        check(activities.size <= MAX_APPS) { "Complete Android app catalog exceeds the entry budget" }
        val targets = linkedMapOf<String, ResolvedLauncherApp>()
        val icons = mutableListOf<ScreenIconReply>()
        var iconBytes = 0
        for (activity in activities) {
            val component = activity.componentName
            val pkg = component.packageName
            val id = "android-launcher:$serial:${component.flattenToString()}"
            check(!targets.containsKey(id)) { "Ambiguous Android launcher component" }
            val info = runCatching { pm.getActivityInfo(component, 0) }.getOrNull()
            val packageInfo = pm.getPackageInfo(pkg, 0)
            val supported = info != null && info.exported && info.enabled && info.applicationInfo.enabled &&
                launcher.isActivityEnabled(component, user) && launcher.isPackageEnabled(pkg, user)
            val name = activity.label.toString().ifBlank { component.shortClassName }
            val packageIcon = AppIcons.reference(context, pkg)
            val content = packageIcon?.let(AppIcons::contentFor)
            val icon = content?.takeIf { iconBytes + it.pngBase64.length <= MAX_ICON_PAYLOAD }?.let {
                // PackageManager proves the source; the actual profile/component
                // identity is bound here, without pretending that a package ID is a user.
                val bound = it.copy(icon = it.icon.copy(appId = id))
                icons += bound
                iconBytes += bound.pngBase64.length
                bound.icon
            }
            val fingerprint = hash(listOf(id, packageInfo.longVersionCode.toString(),
                packageInfo.firstInstallTime.toString(), packageInfo.lastUpdateTime.toString(),
                info?.exported.toString(), info?.enabled.toString(), supported.toString()).joinToString("\u0000"))
            val entry = AndroidInstalledApp(HostCatalogApp(id, name, icon = icon, canLaunch = supported,
                unavailableReason = if (supported) null else "launcher_activity_not_exported_enabled_or_accessible"), fingerprint)
            targets[id] = ResolvedLauncherApp(component, entry)
        }
        check(session(context) == before) { "Android user or lock state changed during catalog enumeration" }
        val limits = listOf("Only this Android user profile is cataloged; no root, Shizuku or cross-user impersonation",
            "Activity start requires an actually resumed, focused, unlocked host activity or local confirmation")
        return LocalAppCatalog(AndroidAppCatalog(before, targets.values.map { it.entry }.sortedBy { it.app.appId }, icons, limits), targets)
    }
}
