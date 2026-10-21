package app.smartexplorer.android.launcher.se

import app.smartexplorer.android.launcher.bridge.ScreenRef
import app.smartexplorer.android.launcher.bridge.StatusCodes
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/**
 * A navigation request in the wire form of `AppNav` (`kind`, `value`): what a tap or a menu entry asks the main
 * process for (U40-11). The launcher turns it into an intent with `AppNav.decode` and `AppNav.intentFor`.
 */
data class NavTarget(val kind: String, val value: String?)

/** The decisions of the tiles, pure Kotlin (U40-11, U40-12). */
object SeActions {
    const val OPEN_LOCATION = "openLocation"
    const val SELECT_TAB = "selectTab"
    const val OPEN_SCREEN = "openScreen"
    const val SHOW_SCREEN_GRID = "showScreenGrid"

    /** The tab of the page „Teilen“ (`MainTab.Share`): Android has no page per device, so the device page is this tab. */
    val SHARE_TAB = NavTarget(SELECT_TAB, "Share")

    /**
     * A tap on a device or a member: waiting for access and a changed identity open the Share tab, since files are not
     * readable without the approval (and never with a changed identity); every other state opens the device's folder.
     */
    fun tapDevice(code: String, location: String): NavTarget =
        if (code == StatusCodes.WAITING_FOR_ACCESS || code == StatusCodes.IDENTITY_CONFLICT) SHARE_TAB else NavTarget(OPEN_LOCATION, folder(location))

    /** `vault://<site>/`: a locked vault shows the unlock sheet in the app. */
    fun tapVault(site: String): NavTarget = NavTarget(OPEN_LOCATION, vaultLocation(site))

    fun vaultLocation(site: String): String = "vault://$site/"

    /** „Alle Bildschirme“, or the screens of the room [profileId]. */
    fun screenGrid(profileId: String? = null): NavTarget = NavTarget(SHOW_SCREEN_GRID, profileId ?: "")

    fun openScreen(screen: ScreenRef, control: Boolean): NavTarget = NavTarget(OPEN_SCREEN, screenJson(screen, control))

    /** `location` and exactly one trailing slash (`share://direct/<id>/`). */
    fun folder(location: String): String = location.trimEnd('/') + "/"

    /**
     * The target JSON of contract V5, the same form as `ScreenTarget.toJson()` of the main process:
     * `{"relation","relation_id","device_id","source","control"}` with `source` null. The launcher builds it itself,
     * since it imports no class of the main process.
     */
    fun screenJson(screen: ScreenRef, control: Boolean): String = buildJsonObject {
        put("relation", screen.relation)
        put("relation_id", screen.relationId)
        put("device_id", screen.deviceId)
        put("source", null as String?)
        put("control", control)
    }.toString()
}
