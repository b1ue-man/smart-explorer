package app.smartexplorer.android.launcher.apps

/**
 * The rules of the private space (API 35, U38-14), pure Kotlin so the JVM tests reach all of them. The private space is
 * a profile the system hides from every app except the default home app that also holds `ACCESS_HIDDEN_PROFILES`.
 */
object PrivateSpace {
    /** The normal permission that, together with the home role, makes the profile visible to the launcher (U35-12). */
    const val PERMISSION = "android.permission.ACCESS_HIDDEN_PROFILES"

    /** The private space exists from Android 15. */
    fun isSupported(sdk: Int): Boolean = sdk >= 35

    /** `getPrivateSpaceSettingsIntent` exists from Android 16 (API 36), not in 15. */
    fun settingsEntryAvailable(sdk: Int): Boolean = sdk >= 36

    /** The launcher sees the private space only with the home role and the permission (R61, DN29). */
    fun isAvailable(sdk: Int, roleHeld: Boolean, permissionGranted: Boolean): Boolean = isSupported(sdk) && roleHeld && permissionGranted

    /** A locked private space is a paused profile; nothing of it is visible, listed or found (U38-14). */
    fun isHidden(kind: ProfileKind, quiet: Boolean): Boolean = kind == ProfileKind.Private && quiet

    /**
     * True when an app of a profile of [kind] belongs in the app list and the search: apps of other profiles always, apps of
     * the private space only while it is unlocked and the person lets it show (the switch in the settings sheet).
     */
    fun appVisible(kind: ProfileKind, quiet: Boolean, showSetting: Boolean): Boolean =
        kind != ProfileKind.Private || (!quiet && showSetting)

    /** The line of the settings card (U35-13, U38-15). */
    fun statusLine(sdk: Int, roleHeld: Boolean, permissionGranted: Boolean): String = when {
        !isSupported(sdk) -> "Privater Bereich: nicht verfügbar (Android 15 oder neuer nötig)"
        !roleHeld -> "Privater Bereich: erst als Standard-Startbildschirm verfügbar"
        !permissionGranted -> "Privater Bereich: Berechtigung fehlt"
        else -> "Privater Bereich: verfügbar"
    }
}
