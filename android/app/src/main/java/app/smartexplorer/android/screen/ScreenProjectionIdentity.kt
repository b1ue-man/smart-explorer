package app.smartexplorer.android.screen

import android.media.projection.MediaProjection

/**
 * The public MediaProjection grant identifies this projection, not a selected package or task.
 * Resize/visibility callbacks expose content facts only. Usage/accessibility app lists and an
 * incoming packageName cannot prove which source the user selected in the system picker.
 */
internal class ScreenProjectionIdentity(private val projection: MediaProjection) {
    private var valid = true
    private var contentSize: Pair<Int, Int>? = null

    @Synchronized fun matches(current: MediaProjection): Boolean = valid && projection === current

    @Synchronized fun resized(current: MediaProjection, width: Int, height: Int): Boolean {
        if (!matches(current) || width <= 0 || height <= 0) return false
        contentSize = width to height
        return true
    }

    @Synchronized fun size(): Pair<Int, Int>? = if (valid) contentSize else null

    @Synchronized fun invalidate() { valid = false; contentSize = null }

    companion object {
        const val SOURCE = "system-selected"
        const val UNVERIFIED_APP = "source_identity_unavailable"

        fun requiresPackageProof(source: String): Boolean = source.startsWith("window:")

        const val APP_EXPLANATION = "Android meldet Smart Explorer nicht, welche App im Systemdialog gewählt wurde. " +
            "Die angefragte App kann deshalb nicht sicher zugeordnet werden. Wähle am Betrachter „Bildschirm“ " +
            "und gib danach am Host im Android-Systemdialog die gewünschte App oder den ganzen Bildschirm frei."
    }
}
