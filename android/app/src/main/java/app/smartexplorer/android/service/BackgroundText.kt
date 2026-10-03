package app.smartexplorer.android.service

import app.smartexplorer.android.api.BgStatus
import app.smartexplorer.android.api.SharePower
import app.smartexplorer.android.api.ShareStatus
import app.smartexplorer.android.system.ShareSnapshot
import app.smartexplorer.android.ui.common.Format

/** Status texts of the background worker, shared by notifications, the Sync page and the settings. */
internal object BackgroundText {
    /** One status line of the settings; [warning] lines name a problem and its next step. */
    data class StatusLine(val text: String, val warning: Boolean = false)

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

    /**
     * Header line of the Sync page (spec F15), e.g. "Periodisch · nächster Lauf ~14:30". A planned
     * time already passed means WorkManager holds the due run until its conditions are met.
     */
    fun summary(mode: String, status: BgStatus?, nextRunMs: Long?, now: Long = System.currentTimeMillis()): String {
        if (mode == BackgroundController.MODE_OFF) return "Hintergrund aus – keine geplanten Jobs"
        val label = modeLabel(mode)
        if (status == null) return label
        if (status.paused) return "$label · ${paused(status)}"
        if (!status.syncEnabled) return "$label · Sync ausgeschaltet"
        status.activeJob?.let { return "$label · läuft: $it" }
        val next = status.nextScheduledRunMs ?: nextRunMs
        return when {
            next != null && next <= now -> "$label · fällig, wartet auf Bedingungen"
            next != null -> "$label · nächster Termin ~${moment(next, now)}"
            mode == BackgroundController.MODE_PERSISTENT && status.daemonRunning -> "$label · aktiv"
            else -> "$label · wartet auf Bedingungen"
        }
    }

    /** Text of the "Dauerbetrieb" notification: "3 Jobs, Share online · läuft: Fotos" (spec F17). */
    fun persistentText(status: BgStatus?, enabledJobs: Int, shareOnline: Boolean): String {
        if (status == null) return "Wird gestartet…"
        return buildString {
            append(if (status.paused) paused(status) else if (enabledJobs == 1) "1 Job" else "$enabledJobs Jobs")
            append(if (shareOnline) ", Share online" else ", Share offline")
            status.activeJob?.let { append(" · läuft: $it") }
                ?: status.nextScheduledRunMs?.let { append(" · nächster Termin ${moment(it)}") }
        }
    }

    /** Text of the "Share erreichbar" notification (spec A7). */
    fun reachableText(share: ShareSnapshot): String = when {
        !share.online -> "Share ist offline"
        share.lanOnly -> "Im lokalen Netz für andere Geräte erreichbar"
        !share.connected -> "Verbindung zum Share-Server wird aufgebaut…"
        share.idleSupported == false -> "Share-Server ohne Ruhemodus – Server aktualisieren"
        else -> "Andere Geräte erreichen dieses Telefon"
    }

    /**
     * Status of "Share im Hintergrund erreichbar" in the settings (spec A1, A7): service, server
     * idle mode, and that scheduled jobs stay periodic (A6). [share] `null` = not loaded.
     */
    fun reachability(
        mode: String,
        reachable: Boolean,
        serviceRunning: Boolean,
        share: ShareStatus?,
        now: Long = System.currentTimeMillis(),
    ): List<StatusLine> {
        val persistent = mode == BackgroundController.MODE_PERSISTENT
        if (!persistent && !reachable) return listOf(StatusLine("Aus: Share ist nur bei geöffneter App erreichbar."))
        if (share == null) return emptyList()
        if (!share.running) {
            return listOf(StatusLine("Share ist offline – der Dienst startet, sobald Share auf der Seite „Teilen“ online ist."))
        }
        val lines = mutableListOf<StatusLine>()
        lines += when {
            !serviceRunning -> StatusLine("Der Hintergrunddienst läuft gerade nicht.", warning = true)
            persistent -> StatusLine("Im Dauerbetrieb ist Share immer im Hintergrund erreichbar.")
            else -> StatusLine("Aktiv – Benachrichtigung „Share erreichbar“.")
        }
        lines += serverLine(share, now)
        if (mode == BackgroundController.MODE_PERIODIC) {
            lines += StatusLine("Geplante Jobs verwenden Jobalarme und WorkManager; Android kann den Start verschieben.")
        }
        return lines
    }

    private fun serverLine(share: ShareStatus, now: Long): StatusLine {
        val power = share.power
        return when {
            share.server == null -> StatusLine("Ohne Share-Server nur im selben WLAN erreichbar; WLAN-Multicast bleibt dafür an.")
            !share.connected -> StatusLine(
                "Nicht mit dem Share-Server verbunden" + (share.lastError?.takeIf { it.isNotBlank() }?.let { ": $it" } ?: "."),
                warning = true,
            )
            power.idleSupported == false -> StatusLine("Share-Server ohne Ruhemodus – Server aktualisieren", warning = true)
            power.idleSupported == true -> StatusLine(idleText(power, now))
            else -> StatusLine("Ruhemodus des Share-Servers wird geprüft…")
        }
    }

    /** "Share-Server mit Ruhemodus · Keepalive alle 3 min · ruht · letzter Kontakt 14:32". */
    private fun idleText(power: SharePower, now: Long): String = buildString {
        append("Share-Server mit Ruhemodus")
        power.keepaliveSecs?.takeIf { it > 0 }?.let { append(" · Keepalive alle ${seconds(it)}") }
        if (power.idleActive) append(" · ruht")
        power.lastServerContactMs?.takeIf { it > 0 }?.let { append(" · letzter Kontakt ${moment(it, now)}") }
    }

    /** "3 min", "2 min 30 s", "45 s". */
    private fun seconds(value: Int): String = when {
        value < 60 -> "$value s"
        value % 60 == 0 -> "${value / 60} min"
        else -> "${value / 60} min ${value % 60} s"
    }
}
