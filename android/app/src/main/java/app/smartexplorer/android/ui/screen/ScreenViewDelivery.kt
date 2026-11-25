package app.smartexplorer.android.ui.screen

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import java.util.Locale
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull

/**
 * What the host's statistics say of „Übertragung“ and the line (user 2026-11-25; `stats.chain` of the core with
 * camelCase keys): [running] the mode it runs (`delivery`; `null` from a host before „Übertragung“, which neither takes
 * a change nor measures), [measuredKbps] the available rate it measured and [measuredAgeMs] how long ago.
 */
data class DeliveryReport(val running: String?, val measuredKbps: Long?, val measuredAgeMs: Long?) {
    companion object {
        /** The report in the body of a `stats` event. */
        fun of(body: JsonObject): DeliveryReport {
            val chain = body["chain"] as? JsonObject
            fun text(key: String) = (chain?.get(key) as? JsonPrimitive)?.contentOrNull
            return DeliveryReport(
                running = text("delivery")?.takeIf { it in ViewerCodecChoice.DELIVERIES },
                measuredKbps = text("measuredKbps")?.toLongOrNull(),
                measuredAgeMs = text("measuredAgeMs")?.toLongOrNull(),
            )
        }
    }
}

/**
 * „Übertragung“ and the measured „Datenrate“ of the Android viewer, with the German texts of the desktop viewer
 * (`screen_delivery.rs`, `Delivery::label`/`hint` of the core). Pure functions only.
 */
object ScreenDelivery {
    /** Why neither mode can be chosen for this session's host. */
    const val UNKNOWN_REASON = "das Gerät kennt „Übertragung“ nicht (ältere Version) und überträgt wie bisher"

    fun label(mode: String): String =
        if (mode == ViewerCodecChoice.DELIVERY_RELIABLE) "TCP" else "UDP"

    fun hint(mode: String): String = if (mode == ViewerCodecChoice.DELIVERY_RELIABLE) {
        "Jedes Bild kommt an; bei langsamer Leitung wird es später, nicht verworfen."
    } else {
        "Ein Bild, das zu spät käme, wird verworfen; das Bild bleibt aktuell."
    }

    /** The name of a mode in the statistics: „Echtzeit“, „Zuverlässig“. */
    fun shortName(mode: String): String = if (mode == ViewerCodecChoice.DELIVERY_RELIABLE) "TCP" else "UDP"

    /** kbit/s as Mbit/s with one decimal and the German comma (`48,2`). */
    fun mbitText(kbps: Long): String = String.format(Locale.ROOT, "%.1f", kbps / 1000.0).replace('.', ',')

    /** „Automatisch“ ([automatic]) of „Datenrate“ with what the host measured. */
    fun automaticText(automatic: String, report: DeliveryReport?): String {
        val kbps = report?.measuredKbps
        return when {
            kbps != null -> "$automatic (gemessen: ${mbitText(kbps)} Mbit/s)"
            report?.running != null -> "$automatic (noch nicht gemessen)"
            else -> automatic
        }
    }

    /** „gemessen 48,2 Mbit/s (vor 3 s)“; `null` before the first measurement. */
    private fun measured(report: DeliveryReport): String? {
        val kbps = report.measuredKbps ?: return null
        val age = report.measuredAgeMs ?: return "gemessen ${mbitText(kbps)} Mbit/s"
        return "gemessen ${mbitText(kbps)} Mbit/s (vor ${age / 1000} s)"
    }

    /** The line under „Neu messen“; [statsSeen]: the host's statistics came. */
    fun measuredLine(report: DeliveryReport?, statsSeen: Boolean): String = when {
        !statsSeen -> "wartet auf die erste Statistik des Geräts"
        report?.running == null -> "das Gerät misst die Leitung nicht (ältere Version)"
        else -> report?.let { measured(it) } ?: "noch nicht gemessen"
    }

    /** The statistics texts of the desktop viewer for the info line; none before the host's statistics. */
    fun infoParts(report: DeliveryReport?, statsSeen: Boolean): List<String> {
        if (!statsSeen) return emptyList()
        val running = report?.running ?: return listOf("Übertragung nicht gemeldet (ältere Version)")
        val rate = report?.let { measured(it) }?.let { "Datenrate $it" } ?: "Datenrate noch nicht gemessen"
        return listOf("Übertragung ${shortName(running)}", rate)
    }
}

/**
 * „Neu messen“ and „Übertragung“ (Echtzeit, Zuverlässig) of the settings menu, below „Datenrate“. A change goes to
 * [onSettings] and then live to a host that names „Übertragung“ in its statistics; for a host before it both entries
 * are disabled with the reason (user rule of 2026-11-24: what is shown must work). [onRemeasure] asks the host to
 * measure the line again.
 */
@Composable
internal fun ColumnScope.DeliveryMenuItems(settings: ViewSettings, onSettings: (ViewSettings) -> Unit, ui: ScreenSessionState, onRemeasure: () -> Unit) {
    val report = ui.delivery
    val statsSeen = ui.stats != null
    val unknown = statsSeen && report?.running == null
    DropdownMenuItem(
        text = {
            Column {
                Text(stringResource(R.string.screen_view_remeasure))
                Text(ScreenDelivery.measuredLine(report, statsSeen), style = MaterialTheme.typography.bodySmall)
            }
        },
        enabled = report?.running != null,
        onClick = onRemeasure,
    )
    Text(
        stringResource(R.string.screen_view_delivery),
        style = MaterialTheme.typography.labelMedium,
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
    )
    ViewerCodecChoice.DELIVERIES.forEach { mode ->
        DropdownMenuItem(
            text = {
                Column {
                    Text(ScreenDelivery.label(mode))
                    Text(if (unknown) ScreenDelivery.UNKNOWN_REASON else ScreenDelivery.hint(mode), style = MaterialTheme.typography.bodySmall)
                }
            },
            leadingIcon = { RadioButton(selected = settings.choice.delivery == mode, onClick = null, enabled = !unknown) },
            enabled = !unknown,
            onClick = { onSettings(settings.copy(choice = settings.choice.copy(delivery = mode))) },
        )
    }
    val running = report?.running
    if (running != null && running != settings.choice.delivery) {
        Text(
            "läuft: ${ScreenDelivery.shortName(running)}",
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
        )
    }
}
