package app.smartexplorer.android.ui.screen

import android.content.Context
import app.smartexplorer.android.api.DecoderOffer
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import java.math.BigDecimal
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.put

/**
 * What this viewer chooses for the picture chain (plan 2026-11-14 `bildkette.md` 4.10, package BK7b): the codec
 * (`auto`, `h264`, `hevc`, `av1`, `sharp` „Scharf (GPU)“, which replaces the switch „Schärfe“ and sets the same `SET_MODE`
 * bit `sharpness`, or `lossless`, which replaces the switch „Verlustfrei“ with the same meaning), the decoder
 * stage (`auto`, `hardware`, `software`), the data rate in bit/s (`null` = automatic: the rate the host measured)
 * and „Übertragung“ (`realtime`, `reliable`, user 2026-11-25). The values are the ones of `screen.viewOpen` (`codec`,
 * `decoder`, `maxRate`, `delivery`).
 */
data class ViewerCodecChoice(
    val codec: String = CODEC_AUTO,
    val decoder: String = DECODER_AUTO,
    val maxRateBps: Long? = null,
    /**
     * „Übertragung“: [DELIVERY_REALTIME] („UDP“, late pictures are given up) or [DELIVERY_RELIABLE]
     * („TCP“, every picture arrives). Live through `screen.viewSetDelivery`, never a new session.
     */
    val delivery: String = DELIVERY_REALTIME,
    /**
     * The stored choice this session's choice stands in for when its host cannot serve that one ([ScreenCodecOffer],
     * user rule of 2026-11-24); `null` when the session runs the stored choice itself.
     */
    val replaces: ViewerCodecChoice? = null,
) {
    /** The old switch „Verlustfrei“: it is the codec choice `lossless`. */
    val lossless: Boolean get() = codec == CODEC_LOSSLESS

    /** „Scharf (GPU)“ (plan 2026-11-17 P9): H.264/HEVC on the host's GPU encoder, `SET_MODE` `sharpness`. */
    val sharp: Boolean get() = codec == CODEC_SHARP

    companion object {
        const val CODEC_AUTO = "auto"
        const val CODEC_H264 = "h264"
        const val CODEC_HEVC = "hevc"
        const val CODEC_AV1 = "av1"
        const val CODEC_SHARP = "sharp"
        const val CODEC_LOSSLESS = "lossless"
        /** The order of the desktop viewer (`CodecSetting::ALL`). */
        val CODECS = listOf(CODEC_AUTO, CODEC_H264, CODEC_HEVC, CODEC_AV1, CODEC_SHARP, CODEC_LOSSLESS)

        /** The host's floor of every data rate (`rate_control::MIN_BITRATE_BPS`): 0,15 Mbit/s. */
        const val MIN_RATE_BPS = 150_000L

        /**
         * Whether the viewer may fall back to the lossless tiles on its own (a host's `encoder_fallback_to_tiles`, a
         * decoder of the phone that cannot run the picture): only under „Automatisch“, and under „Verlustfrei“, whose
         * codec the tiles are. A named codec never gives way to another one (user rule of 2026-11-18).
         */
        fun tilesFallbackAllowed(codec: String): Boolean = codec == CODEC_AUTO || codec == CODEC_LOSSLESS

        /** The choices that name a video codec („Scharf (GPU)“ runs on H.264/HEVC). */
        val VIDEO_CODECS = listOf(CODEC_H264, CODEC_HEVC, CODEC_AV1, CODEC_SHARP)

        /**
         * Whether going to [next] must open a new session instead of `screen.viewSetDecoders`: a session whose first
         * picture group came as tiles ([firstCodec] `tiles`) cannot be switched to video by the host's `SET_DECODERS`,
         * so a video codec chosen then opens the session again with that choice. Codex 3/14: a session opened as
         * „Verlustfrei“ ([openedCodec]) is such a session before its first group is reported, and „Automatisch“
         * out of it is negotiated anew too (the host's plan of a tiles session offers only the tiles again).
         */
        fun reopenForVideo(firstCodec: String?, next: ViewerCodecChoice, openedCodec: String? = null): Boolean {
            val tilesSession = firstCodec == "tiles" || (firstCodec == null && openedCodec == CODEC_LOSSLESS)
            return tilesSession && (next.codec in VIDEO_CODECS || next.codec == CODEC_AUTO)
        }

        const val DECODER_AUTO = "auto"
        const val DECODER_HARDWARE = "hardware"
        const val DECODER_SOFTWARE = "software"
        val DECODERS = listOf(DECODER_AUTO, DECODER_HARDWARE, DECODER_SOFTWARE)

        /** The keys of „Übertragung“ (`Delivery::key` of the core); `realtime` is the default. */
        const val DELIVERY_REALTIME = "realtime"
        const val DELIVERY_RELIABLE = "reliable"
        val DELIVERIES = listOf(DELIVERY_REALTIME, DELIVERY_RELIABLE)

        /** A choice from stored or typed values; a name this version does not know is the automatic one („Echtzeit“). */
        fun of(codec: String?, decoder: String?, maxRateBps: Long?, delivery: String? = null): ViewerCodecChoice = ViewerCodecChoice(
            codec = codec?.takeIf { it in CODECS } ?: CODEC_AUTO,
            decoder = decoder?.takeIf { it in DECODERS } ?: DECODER_AUTO,
            maxRateBps = maxRateBps?.takeIf { it > 0 },
            delivery = delivery?.takeIf { it in DELIVERIES } ?: DELIVERY_REALTIME,
        )

        /**
         * Whether going from the choice a session was opened with to [next] needs a new session: the codec and the
         * decoder are part of the negotiation (E6 of BK6). The switch to `lossless` is the live `SET_MODE` of the old
         * switch; the highest data rate is a live frame as well.
         */
        fun needsReopen(opened: ViewerCodecChoice, next: ViewerCodecChoice): Boolean {
            // A session that runs „Automatisch“ in place of a stored choice its host cannot serve keeps running while the
            // stored choice stays (a new data rate alone, user rule of 2026-11-24).
            val stored = opened.replaces ?: opened
            return next.decoder != stored.decoder || (next.codec != CODEC_LOSSLESS && next.codec != stored.codec)
        }

        /** [bps] in Mbit/s as typed in German (`20`, `7,5`); empty for automatic. */
        fun mbitText(bps: Long?): String = bps?.let {
            BigDecimal(it).movePointLeft(6).stripTrailingZeros().toPlainString().replace('.', ',')
        } ?: ""

        /**
         * Mbit/s as typed (`20`, `7,5` or `7.5`) in bit/s (SI: 1 Mbit/s = 1 000 000 bit/s); `null` for text that
         * is no positive number, lies below [MIN_RATE_BPS] (the host would raise it silently) or does not fit a `Long`.
         * Blank text is automatic and the caller's case.
         */
        fun parseMbit(text: String): Long? {
            val bps = typedBps(text) ?: return null
            return if (bps >= MIN_RATE_BPS) bps else null
        }

        /** The typed text is a positive rate below [MIN_RATE_BPS]: the dialog names the floor instead of the format. */
        fun belowMinimum(text: String): Boolean = typedBps(text)?.let { it < MIN_RATE_BPS } == true

        private fun typedBps(text: String): Long? {
            val mbit = text.trim().replace(',', '.').toDoubleOrNull() ?: return null
            val bps = mbit * 1_000_000.0
            // Rounded, not cut: 0,15 Mbit/s is 150 000 bit/s although 0.15 × 10⁶ is not exact in binary.
            return if (mbit.isFinite() && bps >= 1.0 && bps < 9.0e18) Math.round(bps) else null
        }
    }
}

/** The stored choice (private preferences `screen_view`): kept between sessions, like the desktop viewer's. */
object ScreenViewPrefs {
    private const val FILE = "screen_view"
    private const val KEY_CODEC = "codec"
    private const val KEY_DECODER = "decoder"
    private const val KEY_RATE = "max_rate_bps"
    /** „Übertragung“ (user 2026-11-25); absent in the preferences of an older app: `realtime`. */
    private const val KEY_DELIVERY = "delivery"
    /** The key of the old switch „Verlustfrei“: a stored `true` becomes the codec `lossless`. */
    private const val KEY_LOSSLESS = "lossless"

    fun decode(codec: String?, decoder: String?, rate: Long?, legacyLossless: Boolean, delivery: String? = null): ViewerCodecChoice =
        ViewerCodecChoice.of(codec ?: ViewerCodecChoice.CODEC_LOSSLESS.takeIf { legacyLossless }, decoder, rate, delivery)

    fun load(context: Context): ViewerCodecChoice = try {
        val prefs = context.applicationContext.getSharedPreferences(FILE, Context.MODE_PRIVATE)
        decode(
            prefs.getString(KEY_CODEC, null), prefs.getString(KEY_DECODER, null),
            if (prefs.contains(KEY_RATE)) prefs.getLong(KEY_RATE, 0L) else null,
            prefs.getBoolean(KEY_LOSSLESS, false),
            prefs.getString(KEY_DELIVERY, null),
        )
    } catch (e: RuntimeException) {
        ViewerCodecChoice()
    }

    /** The stored choice as it changes ([save]); tiles that keep running follow it (round 12, B2 / Codex 12/9). */
    @Volatile private var observed: MutableStateFlow<ViewerCodecChoice>? = null

    fun observe(context: Context): StateFlow<ViewerCodecChoice> = synchronized(this) {
        observed ?: MutableStateFlow(load(context)).also { observed = it }
    }

    fun save(context: Context, choice: ViewerCodecChoice) {
        synchronized(this) { observed?.value = choice }
        try {
            context.applicationContext.getSharedPreferences(FILE, Context.MODE_PRIVATE).edit().apply {
                putString(KEY_CODEC, choice.codec)
                putString(KEY_DECODER, choice.decoder)
                if (choice.maxRateBps != null) putLong(KEY_RATE, choice.maxRateBps) else remove(KEY_RATE)
                putString(KEY_DELIVERY, choice.delivery)
                remove(KEY_LOSSLESS)
            }.apply()
        } catch (e: RuntimeException) {
            Unit
        }
    }
}

/** One HEVC or AV1 decoder of MediaCodec, reduced to what the offer needs. */
data class VideoDecoderCaps(val codec: String, val hardware: Boolean, val maxWidth: Int, val maxHeight: Int)

/**
 * Which decoders the viewer offers and runs (B-39, 4.10). H.264 runs on every decoder as before; HEVC and AV1 on a
 * hardware decoder, and on a software decoder of the phone only when „Dekodierer: Software“ is chosen or when the
 * phone has no hardware decoder of that codec and the user named that codec („Codec: HEVC“ or „AV1“), the case in
 * which the negotiation demands it [A: the reading of „die Aushandlung es verlangt“].
 */
object ScreenDecoderPolicy {
    private val VIDEO = listOf(ViewerCodecChoice.CODEC_HEVC, ViewerCodecChoice.CODEC_AV1)

    /** Whether a decoder of this stage passes the choice „Dekodierer“. */
    fun stageAllowed(decoder: String, hardware: Boolean): Boolean = when (decoder) {
        ViewerCodecChoice.DECODER_HARDWARE -> hardware
        ViewerCodecChoice.DECODER_SOFTWARE -> !hardware
        else -> true
    }

    /** Whether a decoder of [codec] in this stage may run; [hardwareExists]: the phone has a hardware decoder of [codec]. */
    fun allows(choice: ViewerCodecChoice, codec: String, hardware: Boolean, hardwareExists: Boolean): Boolean {
        if (!stageAllowed(choice.decoder, hardware)) return false
        if (codec !in VIDEO || hardware || choice.decoder == ViewerCodecChoice.DECODER_SOFTWARE) return true
        return !hardwareExists && choice.codec == codec
    }

    /** The decoders of [candidates] in the stage the choice „Dekodierer“ allows (also the hybrid decoder's H.264 ones). */
    fun <T> inStage(decoder: String, candidates: List<T>, hardware: (T) -> Boolean): List<T> =
        candidates.filter { stageAllowed(decoder, hardware(it)) }

    /** Whether the choice leaves [codec] (HEVC or AV1) open: „Automatisch“, the codec itself, „Scharf (GPU)“ for HEVC. */
    private fun videoOpen(choice: ViewerCodecChoice, codec: String): Boolean = when (choice.codec) {
        ViewerCodecChoice.CODEC_AUTO -> true
        ViewerCodecChoice.CODEC_SHARP -> codec == ViewerCodecChoice.CODEC_HEVC
        else -> choice.codec == codec
    }

    /** The HEVC and AV1 offers: only the codecs the choice leaves open, only the decoders [allows] lets run. */
    fun videoOffers(caps: List<VideoDecoderCaps>, choice: ViewerCodecChoice): List<DecoderOffer> {
        val open = caps.filter { it.codec in VIDEO && videoOpen(choice, it.codec) }
        return open
            .filter { cap -> allows(choice, cap.codec, cap.hardware, caps.any { it.codec == cap.codec && it.hardware }) }
            .map { cap ->
                DecoderOffer(
                    codec = cap.codec, profile = "main", hardware = cap.hardware,
                    maxWidth = cap.maxWidth.takeIf { it > 0 }, maxHeight = cap.maxHeight.takeIf { it > 0 },
                )
            }
            .distinct()
    }

    /**
     * The offers for [choice]: the H.264 decoders in the stage the choice allows, then HEVC and AV1, `tiles` last
     * (the lossless codec is no MediaCodec and stays on every choice).
     */
    fun combine(h264AndTiles: List<DecoderOffer>, video: List<DecoderOffer>, choice: ViewerCodecChoice): List<DecoderOffer> {
        val tiles = h264AndTiles.filter { it.codec == "tiles" }
        val h264 = h264AndTiles.filter { it.codec != "tiles" && stageAllowed(choice.decoder, it.hardware) }
        return h264 + video + tiles
    }
}

/** What the view shows of the picture chain: codec, decoder stage and the answer to „Höchste Datenrate“. */
data class ViewerChainInfo(val codec: String?, val decoderHardware: Boolean?, val decoderName: String?, val rate: RateNote)

/** What the view says about „Höchste Datenrate“. */
sealed interface RateNote {
    /** No limit chosen. */
    data object Automatic : RateNote
    /**
     * The host applies the limit: its statistics name it (`stats.chain.rateCeilingKbps`, as the desktop's
     * `confirmed()`). `applied` of the bridge alone only says that it was sent (Codex 3/22).
     */
    data class Applied(val bps: Long) : RateNote
    /** The host does not know the limit (`unsupported_by_host`): it has no effect. */
    data class Unsupported(val bps: Long) : RateNote
    /** A limit was sent (`applied` of the bridge), the host's statistics do not name it (yet): nothing more is claimed. */
    data class Sent(val bps: Long) : RateNote
}

object ScreenChainInfo {
    /**
     * [rateCeiling] is the `rateCeiling` of the `accepted` event (`automatic`, `applied`, `unsupported_by_host`; `null`
     * before it) or of the latest `rateCeiling` event, [openedBps] the limit the session was opened with, [chosenBps]
     * the limit chosen now; [answered]: [rateCeiling] is the answer to the limit chosen last during the session (r1).
     * [reportedKbps] is the ceiling the host's statistics name (`stats.chain.rateCeilingKbps`, kbit/s; `null` from a
     * host without that report): only it confirms the limit; `applied` means sent (Codex 3/22).
     */
    fun rateNote(rateCeiling: String?, openedBps: Long?, chosenBps: Long?, answered: Boolean = false, reportedKbps: Long? = null): RateNote = when {
        chosenBps == null -> RateNote.Automatic
        rateCeiling == "unsupported_by_host" -> RateNote.Unsupported(chosenBps)
        // The report has kbit/s: it can only confirm a limit of whole kbit/s; another one stays „sent“ (Codex 5/23,
        // 1 000 100 and 1 000 900 bit/s would both read 1000).
        reportedKbps != null && chosenBps % 1000 == 0L && reportedKbps == chosenBps / 1000 -> RateNote.Applied(chosenBps)
        else -> RateNote.Sent(chosenBps)
    }
}

/** One encoder of the host as `accepted` (`encoders`, `encodersV2`) and `screen.probe` (`codecs`) name it. */
data class HostCodec(val codec: String, val profile: String? = null, val hardware: Boolean = false)

/** How an entry of „Codec“ or „Dekodierer“ stands for this pair of devices: [reason] in German, `null` when it runs. */
data class CodecVerdict(val reason: String?, val unknown: Boolean = false)

/** What one session runs with ([choice]) and why it differs from the stored one ([notes], German). */
data class SessionCodec(val choice: ViewerCodecChoice, val notes: List<String>)

/**
 * What the viewer offers in „Codec“ and „Dekodierer“ (user rule of 2026-11-24: „was du anzeigst MUSS funktionieren. end
 * to fucking end, KEINE EINZIGE ausnahme“), as the desktop's `codec_offer.rs`: a codec runs when an encoder of the host
 * pairs with a decoder this phone names for it in the hello (`StreamRequest::chosen_decoders`, `negotiate_chosen`);
 * „Scharf (GPU)“ needs a GPU encoder of H.264 or HEVC, „Automatisch“ and „Verlustfrei“ keep the tiles every host has.
 * Every other entry stays in the menu, disabled, with its German reason under it. A stored choice the host cannot serve
 * runs as „Automatisch“ for that session only ([sessionChoice]); the stored one stays for hosts that serve it.
 */
object ScreenCodecOffer {
    private const val UNKNOWN = "die Kodierer dieses Gastgebers sind noch nicht bekannt"
    private val SHARP = setOf(ViewerCodecChoice.CODEC_H264, ViewerCodecChoice.CODEC_HEVC)
    private val NAMED = setOf(ViewerCodecChoice.CODEC_H264, ViewerCodecChoice.CODEC_HEVC, ViewerCodecChoice.CODEC_AV1)

    /** The encoders of the lists [keys] of [body]; `null` when it names none of them. */
    fun hostCodecs(body: JsonObject, vararg keys: String): List<HostCodec>? {
        val lists = keys.mapNotNull { body[it] as? JsonArray }
        if (lists.isEmpty()) return null
        return lists.flatten().mapNotNull { element ->
            val entry = element as? JsonObject ?: return@mapNotNull null
            val codec = (entry["codec"] as? JsonPrimitive)?.contentOrNull ?: return@mapNotNull null
            val profile = (entry["profile"] as? JsonPrimitive)?.contentOrNull
            HostCodec(codec, profile, (entry["hardware"] as? JsonPrimitive)?.booleanOrNull ?: false)
        }
    }

    /** The decoders the hello names for [codec] and [decoder] out of [offers] (`StreamRequest::chosen_decoders`). */
    fun namedDecoders(codec: String, decoder: String, offers: List<DecoderOffer>): List<DecoderOffer> {
        val named = offers.filter { offer ->
            when (codec) {
                ViewerCodecChoice.CODEC_AUTO -> true
                ViewerCodecChoice.CODEC_LOSSLESS -> offer.codec == "tiles"
                ViewerCodecChoice.CODEC_SHARP -> offer.codec in SHARP
                else -> offer.codec == codec
            }
        }.filter { it.codec == "tiles" || ScreenDecoderPolicy.stageAllowed(decoder, it.hardware) }
        return if (decoder == ViewerCodecChoice.DECODER_HARDWARE) named.filter { it.hardware } else named
    }

    /** Whether [decoder] decodes the stream of [encoder] (`CodecOffer::decodes`; HEVC and AV1 `main` is 4:2:0). */
    private fun decodes(decoder: DecoderOffer, encoder: HostCodec): Boolean =
        decoder.codec == encoder.codec && when (encoder.codec) {
            "tiles" -> true
            ViewerCodecChoice.CODEC_H264 -> decoder.profile == "high" || decoder.profile == encoder.profile
            else -> decoder.profile == "high" || (encoder.profile ?: "main") == (decoder.profile ?: "main")
        }

    /** Whether [encoder] may carry [codec] (`CodecPreference::admits`; „Scharf (GPU)“ on a GPU encoder only). */
    private fun admits(codec: String, encoder: HostCodec): Boolean = when (codec) {
        ViewerCodecChoice.CODEC_AUTO, ViewerCodecChoice.CODEC_LOSSLESS -> encoder.codec != "tiles"
        ViewerCodecChoice.CODEC_SHARP -> encoder.hardware && encoder.codec in SHARP
        else -> encoder.codec == codec
    }

    private fun stage(decoder: String): String = when (decoder) {
        ViewerCodecChoice.DECODER_HARDWARE -> " der Art „Hardware“"
        ViewerCodecChoice.DECODER_SOFTWARE -> " der Art „Software“"
        else -> ""
    }

    private fun viewerMissing(codec: String, decoder: String): String = when (codec) {
        ViewerCodecChoice.CODEC_LOSSLESS -> "die Kacheln dekodiert nur Software (Dekodierer „Hardware“)"
        ViewerCodecChoice.CODEC_AUTO -> "dieses Gerät hat keinen Dekodierer${stage(decoder)}"
        ViewerCodecChoice.CODEC_SHARP -> "dieses Gerät hat keinen H.264- oder HEVC-Dekodierer${stage(decoder)}"
        else -> "dieses Gerät hat keinen ${ScreenCodecNotices.label(codec)}-Dekodierer${stage(decoder)}"
    }

    /**
     * How [codec] under [decoder] stands between a host with the encoders [host] (`null`: not known yet) and this phone,
     * whose offers for that choice are [offers] (`ScreenDecoder.offers(choice)`).
     */
    fun codecVerdict(codec: String, decoder: String, host: List<HostCodec>?, offers: List<DecoderOffer>): CodecVerdict {
        val named = namedDecoders(codec, decoder, offers)
        if (named.isEmpty()) return CodecVerdict(viewerMissing(codec, decoder))
        val tiles = (codec == ViewerCodecChoice.CODEC_AUTO || codec == ViewerCodecChoice.CODEC_LOSSLESS) &&
            named.any { it.codec == "tiles" }
        if (tiles) return CodecVerdict(null)
        if (host == null) return CodecVerdict(UNKNOWN, unknown = true)
        val pairs = host.any { encoder -> admits(codec, encoder) && named.any { decodes(it, encoder) } }
        if (pairs) return CodecVerdict(null)
        return CodecVerdict(
            when {
                codec == ViewerCodecChoice.CODEC_SHARP && host.none { admits(codec, it) } ->
                    "dieser Gastgeber hat keinen GPU-Kodierer für H.264 oder HEVC"
                codec in NAMED && host.none { it.codec == codec } ->
                    "dieser Gastgeber hat keinen ${ScreenCodecNotices.label(codec)}-Kodierer"
                else -> "kein Kodierer dieses Gastgebers passt zu einem Dekodierer${stage(decoder)} dieses Geräts"
            },
        )
    }

    /** Each codec of „Codec“ with its reason under the stored decoder stage of [stored]. */
    fun codecReasons(stored: ViewerCodecChoice, host: List<HostCodec>?, offersFor: (ViewerCodecChoice) -> List<DecoderOffer>): Map<String, String?> =
        ViewerCodecChoice.CODECS.associateWith { codec ->
            val choice = stored.copy(codec = codec, replaces = null)
            codecVerdict(codec, choice.decoder, host, offersFor(choice)).reason
        }

    /** Each stage of „Dekodierer“ with its reason under the codec [running] of the session. */
    fun decoderReasons(running: ViewerCodecChoice, host: List<HostCodec>?, offersFor: (ViewerCodecChoice) -> List<DecoderOffer>): Map<String, String?> =
        ViewerCodecChoice.DECODERS.associateWith { decoder ->
            val choice = running.copy(decoder = decoder, replaces = null)
            codecVerdict(choice.codec, decoder, host, offersFor(choice)).reason
        }

    /**
     * What a session with a host of [host] encoders runs with: [stored] while it runs, else „Automatisch“ in place of the
     * codec, then of the decoder stage, each named in [SessionCodec.notes]; [offersFor] gives this phone's offers.
     */
    fun sessionChoice(stored: ViewerCodecChoice, host: List<HostCodec>?, offersFor: (ViewerCodecChoice) -> List<DecoderOffer>): SessionCodec {
        var choice = stored.copy(replaces = null)
        val notes = mutableListOf<String>()
        val codec = codecVerdict(choice.codec, choice.decoder, host, offersFor(choice))
        if (codec.reason != null && !codec.unknown && choice.codec != ViewerCodecChoice.CODEC_AUTO) {
            notes += "${ScreenCodecNotices.label(choice.codec)}: ${codec.reason} – diese Sitzung: Automatisch"
            choice = choice.copy(codec = ViewerCodecChoice.CODEC_AUTO)
        }
        if (choice.decoder != ViewerCodecChoice.DECODER_AUTO) {
            val stage = codecVerdict(choice.codec, choice.decoder, host, offersFor(choice))
            if (stage.reason != null && !stage.unknown) {
                val name = if (choice.decoder == ViewerCodecChoice.DECODER_HARDWARE) "Hardware" else "Software"
                notes += "Dekodierer „$name“: ${stage.reason} – diese Sitzung: Automatisch"
                choice = choice.copy(decoder = ViewerCodecChoice.DECODER_AUTO)
            }
        }
        val plain = stored.copy(replaces = null)
        return SessionCodec(if (choice == plain) plain else choice.copy(replaces = plain), notes)
    }

    /**
     * The encoders of [target] from its probe (`screen.probe` names them in `codecs`, as `se screen probe`); `null` when
     * it does not answer or shows nothing to this phone.
     */
    suspend fun probe(target: ScreenTarget): List<HostCodec>? {
        val args = buildJsonObject {
            put("relation", target.relation)
            put("relationId", target.relationId)
            target.deviceId?.let { put("deviceId", it) }
        }
        val answer = try {
            Core.call("screen.probe", args) as? JsonObject
        } catch (e: CoreException) {
            null
        } ?: return null
        val supported = (answer["supported"] as? JsonPrimitive)?.booleanOrNull == true
        val view = (answer["view"] as? JsonPrimitive)?.contentOrNull
        return if (supported && view != "off") hostCodecs(answer, "codecs") else null
    }

    /** Whether what a session of [stored] runs with depends on the host's encoders. */
    fun hostMatters(stored: ViewerCodecChoice, offersFor: (ViewerCodecChoice) -> List<DecoderOffer>): Boolean {
        val choice = sessionChoice(stored, null, offersFor).choice
        return codecVerdict(choice.codec, choice.decoder, null, offersFor(choice.copy(replaces = null))).unknown
    }
}
