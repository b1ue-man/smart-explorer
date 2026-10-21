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
 * activity calls it at start and from the `LauncherApps.Callback` and the profile broadcasts. A locked private
 * space (a paused [ProfileKind.Private] profile) has **no** apps here (U38-14); an unlocked one is listed with its
 * kind and shows in the list and the search only while [showPrivateSpace] is on.
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

    /** The switch „Privaten Bereich in der App-Liste zeigen“ (U38-14); the view model keeps it in step with `layout.json`. */
    @Volatile
    var showPrivateSpace: Boolean = true

    /** Apps shown in the list and found by the search (U38-14): those of the private space only while it is unlocked and shown. */
    val visible: List<AppEntry> get() = shown(appsState.value, showPrivateSpace)

    /**
     * Profiles whose widgets the picker offers, the owner's first (U38-10). A locked private space is left out, an
     * unlocked one only while the person lets it show.
     */
    fun profilesForPicker(): List<ProfileInfo> =
        profilesState.value.filter { PrivateSpace.appVisible(it.kind, it.isQuiet, showPrivateSpace) }.sortedBy { !it.isPersonal }

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
            // A locked private space lists nothing: not in the app list, the search or the placement (U38-14).
            if (profile.isHidden) continue
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
        /** The entries of [apps] that the list and the search show (U38-14). */
        fun shown(apps: List<AppEntry>, showPrivate: Boolean): List<AppEntry> =
            apps.filter { PrivateSpace.appVisible(it.kind, quiet = false, showSetting = showPrivate) }

        @Volatile
        private var instance: AppCatalog? = null

        fun get(context: Context): AppCatalog = instance ?: synchronized(this) {
            instance ?: AppCatalog(context.applicationContext).also { instance = it }
        }
    }
}
