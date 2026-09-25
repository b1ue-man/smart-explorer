package app.smartexplorer.android.service

import app.smartexplorer.android.api.BgStatus
import app.smartexplorer.android.ui.common.Format

/** Status texts of the background worker, shared by notifications, the Sync page and the settings. */
internal object BackgroundText {
    fun modeLabel(mode: String): String = when (mode) {
        BackgroundController.MODE_OFF -> "Aus"
        BackgroundController.MODE_PERSISTENT -> "Dauerbetrieb"
        else -> "Periodisch"
    }

    /** "Pausiert bis 18:00" (today), "Pausiert bis 26.09. 06:00", or "Pausiert" without end. */
    fun paused(status: BgStatus, now: Long = System.currentTimeMillis()): String {
        val until = status.pausedUntilMs ?: return "Pausiert"
        return "Pausiert bis ${moment(until, now)}"
    }

    /** Time today as "14:30", otherwise "26.09. 14:30". */
    fun moment(epochMs: Long, now: Long = System.currentTimeMillis()): String =
        if (Format.date(epochMs, now) == Format.date(now, now)) Format.time(epochMs) else "${Format.date(epochMs, now)} ${Format.time(epochMs)}"

    /** Header line of the Sync page (spec F15), e.g. "Periodisch · nächster Lauf ~14:30". */
    fun summary(mode: String, status: BgStatus?, nextRunMs: Long?): String {
        if (mode == BackgroundController.MODE_OFF) return "Hintergrund aus – keine geplanten Jobs"
        val label = modeLabel(mode)
        if (status == null) return label
        if (status.paused) return "$label · ${paused(status)}"
        if (!status.syncEnabled) return "$label · Sync ausgeschaltet"
        status.activeJob?.let { return "$label · läuft: $it" }
        if (mode == BackgroundController.MODE_PERSISTENT) {
            return if (status.daemonRunning) "$label · aktiv" else "$label · startet…"
        }
        return if (nextRunMs != null) "$label · nächster Lauf ~${moment(nextRunMs)}" else "$label · wartet auf Bedingungen"
    }
}
