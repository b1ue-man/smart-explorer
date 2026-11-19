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
    private const val NOTICE = "encoder_notice: "

    /** An input Android did not perform (`screen.hostNotice {inputRefused}`), as `codec_reason_text.rs` says it. */
    private val INPUT_REFUSED = mapOf(
        "the device cancelled a gesture of the input" to "Das Gerät hat eine Geste der Eingabe abgebrochen",
        "the device refused a text of the input (no focused field)" to
            "Das Gerät hat einen Text der Eingabe abgelehnt (kein Eingabefeld im Fokus)",
        "the device refused a system action of the input" to "Das Gerät hat eine Systemaktion der Eingabe abgelehnt",
        "the device did not answer an input in time" to "Das Gerät hat auf eine Eingabe nicht rechtzeitig geantwortet",
    )

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
        "the session was opened for the tiles; reopen it for this codec" ->
            "Diese Sitzung wurde für „Verlustfrei“ geöffnet; sie muss für diesen Codec neu geöffnet werden"
        else -> knownError(reason.trim()) ?: reason.trim()
    }

    /**
     * The host's reasons of the review of 2026-11-19 in German, as `codec_reason_text.rs` of the desktop: „Scharf
     * (GPU)“ without a GPU encoder left (`host_worker_encoding.rs`), an encoder that refused to lower its bit rate
     * (`mf_rate.rs`, `vendor_encode.rs`) and a picture too small for the chosen codec (`host_pipeline_admit.rs`);
     * `null` for any other text.
     */
    fun knownError(text: String): String? {
        val error = text.trim()
        val sharp = "Scharf (GPU): no GPU encoder of "
        // `<name> refused to lower its bit rate from <rate> to <rate> bit/s` (Media Foundation: „the encoder“, OpenH264).
        val lowerFrom = " refused to lower its bit rate from "
        val vendorLower = " refused to lower its bit rate to "
        val small = "the picture of "
        return when {
            error.startsWith(sharp) -> {
                val rest = error.removePrefix(sharp)
                val codec = rest.substringBefore(" is left")
                val inner = rest.substringAfter(" is left", "").trim().removePrefix("(").removeSuffix(")")
                val failed = inner.substringBefore("; software passed over: ").removePrefix("failed: ").trim()
                val passed = inner.substringAfter("; software passed over: ", "").trim()
                buildString {
                    append("Scharf (GPU): kein GPU-Kodierer für ${label(codec)} mehr übrig")
                    if (failed.isNotEmpty() && failed != "[]") append(" (ausgefallen: $failed)")
                    if (passed.isNotEmpty()) append("; Software-Kodierer übergangen, Meldung: $passed")
                }
            }
            // The name is one word group: a longer detail is read from its „: “ on ([detailText]).
            error.contains(lowerFrom) && !error.substringBefore(lowerFrom).contains(": ") -> {
                val name = error.substringBefore(lowerFrom).let { if (it == "the encoder") "der Kodierer" else it }
                val rates = error.substringAfter(lowerFrom).removeSuffix(" bit/s")
                "$name lehnte es ab, seine Bitrate von ${rates.substringBefore(" to ")} auf ${rates.substringAfter(" to ")} bit/s zu senken"
            }
            error.contains(vendorLower) && !error.substringBefore(vendorLower).contains(": ") -> {
                val name = error.substringBefore(vendorLower)
                val rest = error.substringAfter(vendorLower)
                val message = rest.substringAfter(" bit/s", "").removePrefix(":").trim()
                "$name lehnte es ab, seine Bitrate auf ${rest.substringBefore(" bit/s")} bit/s zu senken" +
                    if (message.isEmpty()) "" else "; Meldung: $message"
            }
            error.startsWith(small) ->
                "das Bild mit ${error.removePrefix(small).substringBefore(' ')} Pixeln ist kleiner als die 2×2 Pixel, " +
                    "die der Codec braucht; die Übertragung wartet auf ein größeres"
            else -> null
        }
    }

    /**
     * A notice of the host's encoder (`encoder_notice: <sentence>`, `codec_reason_text.rs::encoder_notice_text`):
     * a device minimum above „Höchste Datenrate“ or a refused parameter in German, another sentence as it came.
     */
    fun noticeText(text: String): String {
        val sentence = text.trim()
        val lowest = "the encoder's lowest bit rate "
        val above = " bit/s is above the highest data rate "
        val parameter = "the encoder refused the parameter "
        val asked = " bit/s is above the bit rate "
        // `codec_reason_text.rs` NOTICE_CONTROL / NOTICE_CONTROL_ABOVE (control bytes above the ceiling).
        val control = "the control stream's "
        val controlAbove = " bit/s are above the highest data rate "
        val german = when {
            sentence.startsWith(lowest) && sentence.contains(above) -> {
                val rest = sentence.removePrefix(lowest)
                "Die niedrigste Bitrate des Kodierers (${rest.substringBefore(above)} bit/s) liegt über der Höchsten " +
                    "Datenrate (${rest.substringAfter(above).substringBefore(" bit/s")} bit/s); weniger Bilder halten die Obergrenze ein"
            }
            sentence.startsWith(lowest) && sentence.contains(asked) -> {
                val rest = sentence.removePrefix(lowest)
                "Die niedrigste Bitrate des Kodierers (${rest.substringBefore(asked)} bit/s) liegt über der verlangten " +
                    "Rate (${rest.substringAfter(asked).substringBefore(" bit/s")} bit/s); er sendet mit seiner niedrigsten Rate, " +
                    "die Höchste Datenrate hält das Datenbudget ein"
            }
            sentence in INPUT_REFUSED -> INPUT_REFUSED.getValue(sentence)
            sentence.startsWith(control) && sentence.contains(controlAbove) -> {
                val rest = sentence.removePrefix(control)
                "Der Steuerkanal (Zeiger, Status, Fensterlisten) braucht allein ${rest.substringBefore(controlAbove)} bit/s, " +
                    "mehr als die Höchste Datenrate (${rest.substringAfter(controlAbove).substringBefore(" bit/s")} bit/s); " +
                    "die Bilder erhalten nur noch die Mindestrate"
            }
            sentence.startsWith(parameter) -> {
                val rest = sentence.removePrefix(parameter)
                val name = rest.substringBefore(": ")
                val detail = rest.substringAfter(": ", "")
                "Der Kodierer lehnte den Parameter $name ab" + if (detail.isEmpty()) "" else "; Meldung: $detail"
            }
            else -> knownError(sentence) ?: "Meldung: $sentence"
        }
        return "Hinweis vom Gerät: $german"
    }

    /** [detail] with a known host reason ([knownError]) at its start or after one of its „: “ in German. */
    private fun detailText(detail: String): String {
        var at = 0
        while (at >= 0 && at < detail.length) {
            knownError(detail.substring(at))?.let { return detail.substring(0, at) + it }
            val next = detail.indexOf(": ", at)
            at = if (next < 0) -1 else next + 2
        }
        return detail
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
            else -> knownError(rest)?.let { ": $it" } ?: ": Meldung: $rest"
        }
    }

    /** The notice in a state reason, an error or an end detail; `null` when [text] names no codec report. */
    fun parse(text: String?, chosen: String): CodecNotice? {
        val reason = text?.trim()?.takeIf { it.isNotEmpty() } ?: return null
        reason.indexOf(UNAVAILABLE).takeIf { it >= 0 }?.let { at ->
            // `chosen_codec_unavailable: <codec>: <reason>` or, from the open, `chosen_codec_unavailable: <reason>`.
            val (codec, why) = codecAndReason(reason.substring(at + UNAVAILABLE.length), chosen)
            if (why.startsWith("the picture of ")) {
                // The chosen codec waits for a larger picture (review 2026-11-19).
                return CodecNotice(chosen, null, "Fehler: ${label(codec)} gewählt, ${knownError(why)}.", error = true)
            }
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
        reason.indexOf(NOTICE).takeIf { it >= 0 }?.let { at ->
            // A notice of the host's encoder ends nothing (`HostStage::take_notice`): no error.
            return CodecNotice(chosen, null, noticeText(reason.substring(at + NOTICE.length)), error = false)
        }
        if (!reason.startsWith(FALLBACK)) return null
        val (running, detail) = reason.removePrefix(FALLBACK).let { rest ->
            val colon = rest.indexOf(':')
            if (colon < 0) rest.trim() to "" else rest.substring(0, colon).trim() to rest.substring(colon + 1).trim()
        }
        if (running.isEmpty()) return null
        val suffix = if (detail.isEmpty()) "" else " (${detailText(detail)})"
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
