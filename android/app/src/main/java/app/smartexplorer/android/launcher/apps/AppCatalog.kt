package app.smartexplorer.android.launcher.apps

import android.content.ComponentName
import android.content.Context
import android.content.pm.ApplicationInfo
import android.content.pm.LauncherActivityInfo
import android.content.pm.LauncherApps
import android.os.UserHandle
import android.os.UserManager
import java.text.Collator
import java.util.Locale
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * An app of any profile in the app list (U35-24). [available] is false for a locked or paused profile,
 * a suspended or disabled app: the list greys it out and refuses to start it.
 */
data class AppEntry(
    val label: String,
    val component: ComponentName,
    val user: UserHandle,
    val userSerial: Long,
    val kind: ProfileKind,
    val available: Boolean,
    val info: LauncherActivityInfo,
    val versionCode: Long,
) {
    val packageName: String get() = component.packageName

    /** `<package>/<class>` as stored in `layout.json`. */
    val componentString: String get() = component.flattenToString()

    val key: String get() = "$componentString#$userSerial"
}

/**
 * All launchable apps of all profiles of this process (U35-24). [reload] reads `LauncherApps`; the
 * activity calls it at start and from the `LauncherApps.Callback`, which tells which package changed.
 * Apps of the private space are listed with [ProfileKind.Private] and left out of [visible] until U38.
 */
class AppCatalog private constructor(private val context: Context) {
    private val launcherApps = context.getSystemService(LauncherApps::class.java)
    private val users = context.getSystemService(UserManager::class.java)
    private val appsState = MutableStateFlow<List<AppEntry>>(emptyList())
    private val profilesState = MutableStateFlow<List<ProfileInfo>>(emptyList())

    /** Every app, sorted for display. */
    val apps: StateFlow<List<AppEntry>> = appsState.asStateFlow()

    val profiles: StateFlow<List<ProfileInfo>> = profilesState.asStateFlow()

    /** Serial numbers of the profiles that were unlocked and not paused at the last [reload]. */
    @Volatile
    var usableProfiles: Set<Long> = emptySet()
        private set

    /** Apps shown in the list and found by the search: not those of the private space (U38). */
    val visible: List<AppEntry> get() = appsState.value.filter { it.kind != ProfileKind.Private }

    /** Profiles whose widgets the picker offers, the owner's first (U38-10); the private space is left out until DSK-M12. */
    fun profilesForPicker(): List<ProfileInfo> = profilesState.value.filter { it.kind != ProfileKind.Private }.sortedBy { !it.isPersonal }

    fun find(component: String, userSerial: Long): AppEntry? =
        appsState.value.firstOrNull { it.componentString == component && it.userSerial == userSerial }

    /** Reads the apps of every profile again. Call off the main thread. */
    fun reload() {
        val profiles = Profiles.load(context)
        val collator = Collator.getInstance(Locale.getDefault()).apply { strength = Collator.PRIMARY }
        val entries = ArrayList<AppEntry>()
        val usableNow = HashSet<Long>()
        for (profile in profiles) {
            val usable = isUsable(profile.user)
            if (usable) usableNow.add(profile.userSerial)
            val activities = try {
                launcherApps.getActivityList(null, profile.user)
            } catch (_: SecurityException) {
                emptyList<LauncherActivityInfo>()
            }
            for (info in activities) {
                val application = info.applicationInfo
                val packageName = info.componentName.packageName
                val available = usable &&
                    launcherApps.isPackageEnabled(packageName, profile.user) &&
                    (application.flags and ApplicationInfo.FLAG_SUSPENDED) == 0
                entries += AppEntry(
                    label = info.label?.toString().orEmpty().ifEmpty { packageName },
                    component = info.componentName,
                    user = profile.user,
                    userSerial = profile.userSerial,
                    kind = profile.kind,
                    available = available,
                    info = info,
                    versionCode = application.longVersionCode,
                )
            }
        }
        entries.sortWith(compareBy<AppEntry, String>(collator) { it.label }.thenBy { it.componentString }.thenBy { it.userSerial })
        usableProfiles = usableNow
        profilesState.value = profiles
        appsState.value = entries
    }

    private fun isUsable(user: UserHandle): Boolean = try {
        users.isUserUnlocked(user) && !users.isQuietModeEnabled(user)
    } catch (_: SecurityException) {
        true
    }

    companion object {
        @Volatile
        private var instance: AppCatalog? = null

        fun get(context: Context): AppCatalog = instance ?: synchronized(this) {
            instance ?: AppCatalog(context.applicationContext).also { instance = it }
        }
    }
}
