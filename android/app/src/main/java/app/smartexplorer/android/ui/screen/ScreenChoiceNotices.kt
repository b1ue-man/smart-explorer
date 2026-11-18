package app.smartexplorer.android.ui.screen

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull

/**
 * What the host says about the codec the viewer chose (user rule of 2026-11-18: a choice is used exactly as made, an
 * automatic change only under „Automatisch“, a choice that cannot run is a visible error with its reason). [chosen] is
 * the viewer's codec choice, [running] the codec key the host sends now (`null` when it sends none), [error] whether the
 * choice is not in force.
 */
data class CodecNotice(val chosen: String, val running: String?, val text: String, val error: Boolean)

/**
 * The host's codec reports as the viewer shows them: `encoder_fallback_to_<codec>: …` (another encoder or, under
 * „Automatisch“, another codec, `host_pipeline_produce.rs`), `chosen_codec_unavailable: <codec>: <reason>` (no pairing of
 * the chosen codec, `negotiate_chosen.rs`) and `chosen_codec_failed: <codec>: …` (no encoder of the chosen codec left,
 * `host_fallback.rs`). The texts are pure so a JVM test reads them.
 */
object ScreenCodecNotices {
    private const val FALLBACK = "encoder_fallback_to_"
    private const val UNAVAILABLE = "chosen_codec_unavailable: "
    private const val FAILED = "chosen_codec_failed: "

    /** The label of a codec key of the viewer or the host (`tiles` is „Verlustfrei“). */
    fun label(codec: String): String = when (codec) {
        ViewerCodecChoice.CODEC_AUTO -> "Automatisch"
        ViewerCodecChoice.CODEC_H264 -> "H.264"
        ViewerCodecChoice.CODEC_HEVC -> "HEVC"
        ViewerCodecChoice.CODEC_AV1 -> "AV1"
        ViewerCodecChoice.CODEC_SHARP -> "Scharf (GPU)"
        ViewerCodecChoice.CODEC_LOSSLESS, "tiles" -> "Verlustfrei"
        else -> codec
    }

    /**
     * Whether the host's [running] codec is the [chosen] one: another encoder of the same codec may stand in, „Scharf
     * (GPU)“ runs on H.264 or HEVC, „Verlustfrei“ on the tiles, „Automatisch“ on anything.
     */
    fun belongs(chosen: String, running: String): Boolean = when (chosen) {
        ViewerCodecChoice.CODEC_AUTO -> true
        ViewerCodecChoice.CODEC_SHARP -> running == ViewerCodecChoice.CODEC_H264 || running == ViewerCodecChoice.CODEC_HEVC
        ViewerCodecChoice.CODEC_LOSSLESS -> running == "tiles"
        else -> running == chosen
    }

    /** The host's English reasons of `chosen_codec_unavailable` in German; another one stays as it came. */
    private fun reasonText(reason: String): String = when (reason.trim()) {
        "the host has no encoder of this codec" -> "Das Gerät hat keinen Kodierer dieses Codecs"
        "the host has no GPU encoder of H.264 or HEVC" -> "Das Gerät hat keinen GPU-Kodierer für H.264/HEVC"
        "the viewer has no decoder of this codec" -> "Dieses Handy bietet keinen Dekodierer dieses Codecs an"
        "no encoder and decoder of this codec fit together" -> "Kein Kodierer und Dekodierer dieses Codecs passen zusammen"
        else -> reason.trim()
    }

    /**
     * The reason of `chosen_codec_failed` in German (measurement of 2026-11-18, as `codec_reason_text.rs` of the
     * desktop): `no encoder of the chosen codec is left[: <error>]`; a driver's own message follows as „Meldung: …“.
     */
    private fun failedReason(reason: String): String {
        val rest = reason.trim().removePrefix("no encoder of the chosen codec is left").trim().removePrefix(":").trim()
        val firstPicture = "no first picture from the encoder within "
        val noPairing = "no pairing carries "
        return when {
            rest.isEmpty() -> ""
            rest.startsWith(firstPicture) ->
                ": der Kodierer lieferte ${rest.removePrefix(firstPicture).removeSuffix(" ms").trim()} ms lang kein erstes Bild"
            rest.startsWith(noPairing) ->
                ": keine Kodierer-Paarung schafft ${rest.removePrefix(noPairing).substringBefore(' ')} mit einem Bild pro Sekunde"
            else -> ": Meldung: $rest"
        }
    }

    /** The notice in a state reason, an error or an end detail; `null` when [text] names no codec report. */
    fun parse(text: String?, chosen: String): CodecNotice? {
        val reason = text?.trim()?.takeIf { it.isNotEmpty() } ?: return null
        reason.indexOf(UNAVAILABLE).takeIf { it >= 0 }?.let { at ->
            // `chosen_codec_unavailable: <codec>: <reason>` or, from the open, `chosen_codec_unavailable: <reason>`.
            val (codec, why) = codecAndReason(reason.substring(at + UNAVAILABLE.length), chosen)
            return CodecNotice(
                chosen, null,
                "Fehler: Der gewählte Codec „${label(codec)}“ ist nicht verfügbar: ${reasonText(why)}. Es wird kein anderer Codec gesendet.",
                error = true,
            )
        }
        reason.indexOf(FAILED).takeIf { it >= 0 }?.let { at ->
            val (codec, why) = codecAndReason(reason.substring(at + FAILED.length), chosen)
            return CodecNotice(
                chosen, null,
                "Fehler: ${label(codec)} gewählt, aber alle Kodierer dieses Codecs am Gerät sind ausgefallen${failedReason(why)}. Es wird kein anderer Codec gesendet.",
                error = true,
            )
        }
        if (!reason.startsWith(FALLBACK)) return null
        val (running, detail) = reason.removePrefix(FALLBACK).let { rest ->
            val colon = rest.indexOf(':')
            if (colon < 0) rest.trim() to "" else rest.substring(0, colon).trim() to rest.substring(colon + 1).trim()
        }
        if (running.isEmpty()) return null
        val suffix = if (detail.isEmpty()) "" else " ($detail)"
        return if (belongs(chosen, running)) {
            CodecNotice(chosen, running, "Codec gewählt: ${label(chosen)} · läuft: ${label(running)}$suffix", error = false)
        } else {
            CodecNotice(
                chosen, running,
                "Fehler: Der gewählte Codec „${label(chosen)}“ wird nicht gesendet, das Gerät ist auf „${label(running)}“ ausgewichen$suffix.",
                error = true,
            )
        }
    }

    /**
     * The phone's decoder cannot run the host's picture ([reason] of the decoder) while the [chosen] codec forbids the
     * automatic switch to the tiles ([ViewerCodecChoice.tilesFallbackAllowed]): an error instead of another codec.
     */
    fun decoderUnavailable(chosen: String, decoder: String, reason: String): String {
        val stage = when (decoder) {
            ViewerCodecChoice.DECODER_HARDWARE -> " mit „Dekodierer: Hardware“"
            ViewerCodecChoice.DECODER_SOFTWARE -> " mit „Dekodierer: Software“"
            else -> ""
        }
        return "Fehler: Dieses Handy kann den gewählten Codec „${label(chosen)}“$stage nicht dekodieren: ${reason.take(512)}. " +
            "Es wird nicht auf „Verlustfrei“ ausgewichen; bitte einen anderen Codec oder Dekodierer wählen."
    }

    /** The keys a report may name its codec by (the viewer's choices and the host's `CodecPreference`/scope keys). */
    private val CODEC_KEYS = ViewerCodecChoice.CODECS + "tiles"

    /** `<codec>: <reason>` into its parts; a text without a leading codec key is the reason of the [chosen] codec. */
    private fun codecAndReason(text: String, chosen: String): Pair<String, String> {
        val colon = text.indexOf(':')
        val head = if (colon < 0) text.trim() else text.substring(0, colon).trim()
        return when {
            head in CODEC_KEYS && colon >= 0 -> head to text.substring(colon + 1).trim()
            head in CODEC_KEYS -> head to ""
            else -> chosen to text.trim()
        }
    }
}

/**
 * The state line of „Monitor“ and „Auflösung“ (plan 2026-11-17 P5/P9, as the desktop viewer's `state_line` and
 * `VirtualDisplayReport::text`): what the viewer chose, what the host did (`virtualDisplay` of the `accepted` event) and
 * why, a size the host set other than the chosen one (`requested`, `adjusted`: „gewählt X, eingestellt Y, Grund: …“), a
 * monitor it moved the session to (`source`, `sourceChosen`: „Quelle: <gewählt> → jetzt <tatsächlich>“), a window source
 * the wish does not apply to (`source_not_monitor`), and what it does now when the statistics name another state
 * (`chain.virtualDisplay`, `VirtualDisplayReport::chain_key`). A host without a report does not support the choice.
 */
object ScreenDisplayReportText {
    private const val SOURCE_NOT_MONITOR = "source_not_monitor"
    private const val NOT_MONITOR_TEXT = "Monitor-Wunsch gilt nur für Monitor-Quellen, die gewählte Quelle hat Vorrang"

    /** The wish as the desktop viewer's `wish_text` names it. */
    fun wishText(wish: String): String = when (wish) {
        ViewerDisplayChoice.WISH_OFF -> "physisch"
        ViewerDisplayChoice.WISH_REQUEST -> "eigener (virtuell)"
        else -> "Automatisch"
    }

    private fun str(obj: JsonObject, key: String): String? = (obj[key] as? JsonPrimitive)?.contentOrNull

    private fun int(obj: JsonObject, key: String): Int = (obj[key] as? JsonPrimitive)?.intOrNull ?: 0

    private fun sizeOf(obj: JsonObject?): Pair<Int, Int>? {
        if (obj == null) return null
        val width = int(obj, "width")
        val height = int(obj, "height")
        return if (width > 0 && height > 0) width to height else null
    }

    private fun sizeText(size: Pair<Int, Int>): String = "${size.first}×${size.second}"

    /** `adjust_text`: the German reason of `adjusted` (`refused: <error>` keeps its error). */
    fun adjustText(why: String): String {
        val colon = why.indexOf(':')
        val key = if (colon < 0) why.trim() else why.substring(0, colon).trim()
        val detail = if (colon < 0) "" else why.substring(colon + 1).trim()
        val text = when (key) {
            "mode_rules" -> "Regeln der Anzeigemodi (mindestens 640×360 im selben Seitenverhältnis, Breite ein Vielfaches von 8, " +
                "höchstens 16384 je Seite und die Pixelgrenze der Aufnahme)"
            "driver_limit" -> "Grenze des Anzeigetreibers"
            "other_session" -> "eine andere Sitzung bestimmt die Größe (die mit Steuerung, sonst die neueste)"
            "pending" -> "die Änderung folgt nach dem kürzesten Abstand zwischen zwei Änderungen"
            "driver_set_other" -> "der Anzeigetreiber stellte eine andere Größe ein oder lehnte sie ab"
            "physical_nearest_mode" -> "der Monitor kennt diese Größe nicht, sein nächstliegender Modus gilt"
            "refused" -> "der Monitor lehnte den Modus ab"
            SOURCE_NOT_MONITOR -> "die gewählte Quelle ist kein Monitor"
            else -> key
        }
        return if (detail.isEmpty()) text else "$text: $detail"
    }

    /** `VirtualDisplayReport::text` of the host's report, with the size wherever it names one. */
    fun reportText(report: JsonObject): String {
        val size = sizeOf(report)
        val shown = size?.let(::sizeText) ?: "?"
        val at = size?.let { ", ${sizeText(it)}" } ?: ""
        val driver = str(report, "driver").orEmpty()
        val reason = str(report, "reason")
        val text = when (str(report, "state")) {
            "inactive" -> when (reason) {
                "physical_monitor" -> "aus (Monitor am Gerät$at)"
                "off" -> "aus (physischer Monitor gewählt$at)"
                "available" -> "bereit ($driver$at)"
                "setup_needed" -> "bereit nach einmaliger Einrichtung mit Freigabe"
                "physical_mode_refused" -> "aus (der Monitor lehnte die gewählte Auflösung ab$at)"
                SOURCE_NOT_MONITOR -> "aus ($NOT_MONITOR_TEXT$at)"
                null -> size?.let { "aus (${sizeText(it)})" } ?: "aus"
                else -> "aus ($reason$at)"
            }
            "active" -> "$shown ($driver)"
            "offered" -> "$shown am Portal angeboten, die Sitzung entscheidet"
            "setup" -> "wird eingerichtet (Freigabe des Betriebssystems)"
            "after_restart" -> "eingerichtet, wirkt nach Abmeldung oder Neustart"
            "unsupported" -> if (reason == SOURCE_NOT_MONITOR) "nicht möglich ($NOT_MONITOR_TEXT)" else "nicht möglich (${reason ?: "unbekannt"})"
            else -> "unbekannter Zustand"
        }
        val line = StringBuilder("Virtueller Bildschirm: $text")
        sizeOf(report["requested"] as? JsonObject)?.let { requested ->
            val why = str(report, "adjusted")?.let(::adjustText).orEmpty()
            line.append(" · gewählt ${sizeText(requested)}, eingestellt ${size?.let(::sizeText) ?: "keine"}, Grund: $why")
        }
        str(report, "source")?.let { source ->
            val chosen = str(report, "sourceChosen")
            line.append(if (chosen != null) " · Quelle: $chosen → jetzt $source" else " · Quelle: jetzt $source")
        }
        return line.toString()
    }

    /** `VirtualDisplayReport::chain_key`: the spelling of the statistics, with `;requested=…;adjusted=…` and `;source=…`. */
    fun chainKey(report: JsonObject): String {
        val reason = str(report, "reason") ?: "-"
        val dims = "${int(report, "width")}x${int(report, "height")}"
        val key = StringBuilder(when (val state = str(report, "state")) {
            "active", "offered" -> "$state:$dims:${str(report, "driver").orEmpty()}"
            "unsupported", "inactive" -> "$state:$reason"
            "setup", "after_restart" -> state
            else -> "other"
        })
        (report["requested"] as? JsonObject)?.let { requested ->
            val why = (str(report, "adjusted") ?: "-").substringBefore(':')
            key.append(";requested=${int(requested, "width")}x${int(requested, "height")};adjusted=$why")
        }
        str(report, "source")?.let { key.append(";source=$it") }
        return key.toString()
    }

    /** The whole line: chosen, actual (with the reason) and, when the statistics differ, the state now. */
    fun line(chosen: ViewerDisplayChoice, report: JsonObject?, now: String?): String {
        val wanted = "Gewählt: Monitor ${wishText(chosen.wish)}, Auflösung ${chosen.sizeText ?: "folgt der Ansicht"}"
        val actual = when {
            report == null -> "vom Gerät nicht unterstützt"
            now != null && now != chainKey(report) -> "${reportText(report)} · jetzt $now"
            else -> reportText(report)
        }
        return "$wanted · Tatsächlich: $actual"
    }
}
