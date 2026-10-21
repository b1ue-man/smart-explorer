package app.smartexplorer.android.launcher

/** What the system currently answers for the HOME intent (U35-8). */
sealed interface HomeResolution {
    /** Smart Explorer's own launcher is the default. */
    data object Own : HomeResolution

    /** The system selection dialog (no default chosen). */
    data object Chooser : HomeResolution

    /** Another launcher; [componentName] is `<package>/<class>`. */
    data class Other(val componentName: String, val label: String) : HomeResolution

    /** Nothing answers (no launcher installed). */
    data object None : HomeResolution
}

/** What the settings card says about the home screen after deactivation (U35-9). */
sealed interface HomeNotice {
    /** „Startbildschirm: <label>“. */
    data class Current(val label: String) : HomeNotice

    /** „Wähle deinen Startbildschirm. Vorher: <name>.“ with the two buttons. */
    data class ChooseAgain(val previousHome: String) : HomeNotice

    data object Empty : HomeNotice
}

/** Pure decisions of the launcher switch; the Android side only feeds them (U35-8, U35-9). */
internal object HomeRules {
    /** Packages of the system selection activity: `android` before Android 14, `com.android.intentresolver` since (A46). */
    val CHOOSER_PACKAGES = setOf("android", "com.android.intentresolver")

    /** Classifies the answer to `resolveActivity(HOME, MATCH_DEFAULT_ONLY)`. */
    fun classify(packageName: String?, className: String?, ownPackage: String, label: () -> String): HomeResolution = when {
        packageName == null -> HomeResolution.None
        packageName == ownPackage -> HomeResolution.Own
        packageName in CHOOSER_PACKAGES -> HomeResolution.Chooser
        else -> HomeResolution.Other("$packageName/${className.orEmpty()}", label())
    }

    /**
     * The card's statement while the launcher is off, determined anew on every `onResume` and never
     * stored (U35-9): the same launcher as before → its name; otherwise, while the notice was not
     * dismissed and a previous launcher is known → the request to choose; another launcher without
     * that → its name; else nothing.
     */
    fun notice(previousHome: String?, noticeDismissed: Boolean, home: HomeResolution): HomeNotice = when {
        previousHome != null && home is HomeResolution.Other && home.componentName == previousHome ->
            HomeNotice.Current(home.label)
        previousHome != null && !noticeDismissed -> HomeNotice.ChooseAgain(previousHome)
        home is HomeResolution.Other -> HomeNotice.Current(home.label)
        else -> HomeNotice.Empty
    }
}
