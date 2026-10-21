package app.smartexplorer.android.launcher.apps

import android.app.role.RoleManager
import android.content.Context
import android.content.pm.LauncherApps
import android.content.pm.PackageManager
import android.os.Build
import android.os.Process
import android.os.UserHandle
import android.os.UserManager

enum class ProfileKind { Personal, Work, Private, Other }

/**
 * One profile of the person (U35-24): [isPersonal] the one the launcher runs in, [isWork] a managed
 * (work) profile, [userSerial] the number stored in `layout.json`. [isQuiet] is the paused state
 * (`UserManager.isQuietModeEnabled`, U38-13): for the work profile „pausiert“, for the private space (API 35,
 * [ProfileKind.Private]) „gesperrt“ (U38-14).
 */
data class ProfileInfo(val user: UserHandle, val userSerial: Long, val kind: ProfileKind, val isQuiet: Boolean = false) {
    val isPersonal: Boolean get() = kind == ProfileKind.Personal
    val isWork: Boolean get() = kind == ProfileKind.Work
    val isPrivate: Boolean get() = kind == ProfileKind.Private

    /** A locked private space: nothing of it shows (U38-14). */
    val isHidden: Boolean get() = PrivateSpace.isHidden(kind, isQuiet)
}

object Profiles {
    fun load(context: Context): List<ProfileInfo> {
        val launcherApps = context.getSystemService(LauncherApps::class.java)
        val users = context.getSystemService(UserManager::class.java)
        val self = Process.myUserHandle()
        val privateAllowed = privateSpaceAvailable(context)
        return launcherApps.profiles.mapNotNull { user ->
            val kind = kindOf(launcherApps, user, self)
            // The private space counts only with the home role and the permission (U38-14).
            if (kind == ProfileKind.Private && !privateAllowed) return@mapNotNull null
            ProfileInfo(user, users.getSerialNumberForUser(user), kind, quiet(users, user))
        }
    }

    private fun quiet(users: UserManager, user: UserHandle): Boolean = try {
        users.isQuietModeEnabled(user)
    } catch (_: SecurityException) {
        false
    }

    /** True when the private space may be shown: API 35, the home role held and `ACCESS_HIDDEN_PROFILES` granted (U38-14). */
    fun privateSpaceAvailable(context: Context): Boolean = PrivateSpace.isAvailable(
        Build.VERSION.SDK_INT,
        context.getSystemService(RoleManager::class.java)?.isRoleHeld(RoleManager.ROLE_HOME) == true,
        context.checkSelfPermission(PrivateSpace.PERMISSION) == PackageManager.PERMISSION_GRANTED,
    )

    private fun kindOf(launcherApps: LauncherApps, user: UserHandle, self: UserHandle): ProfileKind {
        if (user == self) return ProfileKind.Personal
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.VANILLA_ICE_CREAM) {
            when (launcherApps.getLauncherUserInfo(user)?.userType) {
                UserManager.USER_TYPE_PROFILE_MANAGED -> return ProfileKind.Work
                UserManager.USER_TYPE_PROFILE_PRIVATE -> return ProfileKind.Private
                UserManager.USER_TYPE_PROFILE_CLONE -> return ProfileKind.Other
                else -> Unit
            }
        }
        // Before API 35 the profiles a launcher is shown are the person's own and managed (work) ones.
        return ProfileKind.Work
    }

    /** Serial number of the launcher's own profile. */
    fun selfSerial(context: Context): Long = context.getSystemService(UserManager::class.java).getSerialNumberForUser(Process.myUserHandle())
}
