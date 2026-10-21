package app.smartexplorer.android.launcher.apps

import android.content.Context
import android.content.pm.LauncherApps
import android.os.Build
import android.os.Process
import android.os.UserHandle
import android.os.UserManager

enum class ProfileKind { Personal, Work, Private, Other }

/**
 * One profile of the person (U35-24): [isPersonal] the one the launcher runs in, [isWork] a managed
 * (work) profile, [userSerial] the number stored in `layout.json`. The private space (API 35) is
 * recognised as [ProfileKind.Private] but not handled here; U38 does that.
 */
data class ProfileInfo(val user: UserHandle, val userSerial: Long, val kind: ProfileKind) {
    val isPersonal: Boolean get() = kind == ProfileKind.Personal
    val isWork: Boolean get() = kind == ProfileKind.Work
}

object Profiles {
    fun load(context: Context): List<ProfileInfo> {
        val launcherApps = context.getSystemService(LauncherApps::class.java)
        val users = context.getSystemService(UserManager::class.java)
        val self = Process.myUserHandle()
        return launcherApps.profiles.map { user ->
            ProfileInfo(user, users.getSerialNumberForUser(user), kindOf(launcherApps, user, self))
        }
    }

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
