package app.smartexplorer.android.screen

import android.media.MediaCodecInfo.CodecProfileLevel
import android.media.MediaFormat
import java.nio.ByteBuffer

/**
 * The codecs of the phone's MediaCodec encoder (plan 2026-11-14 `bildkette.md`, B-26, package BK7b): which MIME type and
 * profile a codec name of the core's `configure` means, which encoders may run it, and where the parameter sets
 * of a key access unit stand. Everything here is plain bytes and constants, so a JVM test can check it.
 *
 * H.264 runs on every encoder, HEVC and AV1 only on a hardware encoder (`MediaCodecInfo.isHardwareAccelerated`,
 * API 29): a software HEVC or AV1 encoder of the phone would take the CPU the picture chain needs for other work and
 * the core ranks H.264 before software AV1 anyway (B-30). The phone reports 4:2:0 (`main`) only: Android names no
 * profile constant for HEVC 4:4:4 (RExt) and none for AV1 `High` [A, Android reference, MediaCodecInfo.CodecProfileLevel].
 */
internal object ScreenEncoderCodecs {
    const val H264 = "h264"
    const val HEVC = "hevc"
    const val AV1 = "av1"

    /** `MediaFormat.MIMETYPE_VIDEO_HEVC`: API 21. */
    const val MIME_HEVC = MediaFormat.MIMETYPE_VIDEO_HEVC
    /** `MediaFormat.MIMETYPE_VIDEO_AV1`: API 29. */
    const val MIME_AV1 = MediaFormat.MIMETYPE_VIDEO_AV1

    /** The video codecs the phone reports beside H.264, in the order of its offers. */
    val VIDEO = listOf(HEVC, AV1)

    fun mime(codec: String): String? = when (codec) {
        H264 -> MediaFormat.MIMETYPE_VIDEO_AVC
        HEVC -> MIME_HEVC
        AV1 -> MIME_AV1
        else -> null
    }

    /**
     * The `KEY_PROFILE` value of the core's profile name: H.264 `cb`/`high`, HEVC `main` (`HEVCProfileMain`, API 21) and
     * AV1 `main` (`AV1ProfileMain8`, API 29); `null` for a name the phone never offers.
     */
    fun profile(codec: String, key: String): Int? = when (codec) {
        H264 -> if (key == "cb") CodecProfileLevel.AVCProfileConstrainedBaseline else CodecProfileLevel.AVCProfileHigh
        HEVC -> if (key == "main") CodecProfileLevel.HEVCProfileMain else null
        AV1 -> if (key == "main") CodecProfileLevel.AV1ProfileMain8 else null
        else -> null
    }

    /** Whether only a hardware encoder may run [codec]. */
    fun hardwareOnly(codec: String): Boolean = codec != H264

    /** The offer name of a profile: `main` for HEVC and AV1. */
    fun offerProfile(codec: String): String = if (codec == H264) "high" else "main"

    // --- Annex B (H.264, HEVC) -----------------------------------------------------------------------------------

    /** The first byte after each start code `00 00 01` of [unit] (its position to limit). */
    private fun nalHeaders(unit: ByteBuffer): List<Int> {
        val at = unit.position()
        val end = unit.limit()
        val found = ArrayList<Int>()
        var index = at
        while (index + 3 < end) {
            if (unit.get(index) == 0.toByte() && unit.get(index + 1) == 0.toByte() && unit.get(index + 2) == 1.toByte()) {
                found.add(unit.get(index + 3).toInt() and 255)
                index += 3
            } else index++
        }
        return found
    }

    /** The HEVC NAL unit types of [unit] (H.265 7.3.1.2: `(header >> 1) & 63`). */
    fun hevcTypes(unit: ByteBuffer): List<Int> = nalHeaders(unit).map { (it shr 1) and 63 }

    const val HEVC_IDR_W_RADL = 19
    const val HEVC_IDR_N_LP = 20
    const val HEVC_VPS = 32
    const val HEVC_SPS = 33
    const val HEVC_PPS = 34

    /** Whether [unit] holds all three HEVC parameter sets. */
    fun hevcHasParameterSets(unit: ByteBuffer): Boolean {
        val types = hevcTypes(unit)
        return HEVC_VPS in types && HEVC_SPS in types && HEVC_PPS in types
    }

    // --- AV1 low-overhead bitstream (AV1 5.3, 5.5) ---------------------------------------------------------------

    const val OBU_SEQUENCE_HEADER = 1
    const val OBU_TEMPORAL_DELIMITER = 2
    const val OBU_FRAME_HEADER = 3
    const val OBU_FRAME = 6

    /** One OBU of a temporal unit: its type, its byte range (header to end of payload), its payload start and `obu_has_size_field`. */
    data class Obu(val type: Int, val start: Int, val end: Int, val payloadStart: Int = start, val hasSize: Boolean = true)

    /** The OBUs of [unit], `null` when the sizes do not fit (a damaged or foreign stream). */
    fun av1Obus(unit: ByteBuffer): List<Obu>? {
        var at = unit.position()
        val end = unit.limit()
        val found = ArrayList<Obu>()
        while (at < end) {
            val header = unit.get(at).toInt() and 255
            if (header and 0x80 != 0) return null
            val type = (header shr 3) and 15
            val extension = (header shr 2) and 1
            val hasSize = (header shr 1) and 1
            var cursor = at + 1 + extension
            if (cursor > end) return null
            val payloadEnd: Int
            if (hasSize == 1) {
                // AV1 4.10.5 (`leb128`): up to 8 bytes (including zero-padded hardware sizes), value <= 2^32 - 1.
                var size = 0L
                var bytes = 0
                while (true) {
                    if (cursor >= end || bytes >= 8) return null
                    val byte = unit.get(cursor++).toInt() and 255
                    size = size or ((byte and 0x7F).toLong() shl (bytes * 7))
                    bytes++
                    if (byte and 0x80 == 0) break
                }
                if (size > end - cursor) return null
                payloadEnd = cursor + size.toInt()
            } else payloadEnd = end
            found.add(Obu(type, at, payloadEnd, cursor, hasSize == 1))
            at = payloadEnd
        }
        return found
    }

    fun av1HasSequenceHeader(unit: ByteBuffer): Boolean = av1Obus(unit)?.any { it.type == OBU_SEQUENCE_HEADER } == true

    /** Whether [unit] holds an AV1 `KEY_FRAME` (`show_existing_frame == 0`, `frame_type == 0`, AV1 5.9.2; matches `video_keyframe`). */
    fun av1IsKeyUnit(unit: ByteBuffer): Boolean {
        val frame = av1Obus(unit)?.firstOrNull { it.type == OBU_FRAME || it.type == OBU_FRAME_HEADER } ?: return false
        return frame.payloadStart < frame.end && (unit.get(frame.payloadStart).toInt() and 0xE0) == 0
    }

    /** The offset in [unit] behind a leading temporal delimiter OBU, 0 without one (the sequence header goes there). */
    fun av1AfterTemporalDelimiter(unit: ByteBuffer): Int {
        val first = av1Obus(unit)?.firstOrNull() ?: return 0
        return if (first.type == OBU_TEMPORAL_DELIMITER && first.hasSize) first.end - unit.position() else 0
    }

    /**
     * The configuration OBUs of the encoder's `csd-0`: the AV1 codec configuration record (`av1C`, ISO/IEC 14496-15:
     * 4 bytes, the first is marker 1 and version 1 = 0x81) is cut off, an encoder that gives the bare OBUs keeps them.
     * The record may end after its 4 bytes (its `configOBUs` are optional): the result is then empty, nothing is
     * stored, and the key frame carries its own sequence header. No OBU starts with 0x81 (its first bit is forbidden).
     */
    fun av1ConfigObus(csd: ByteBuffer): ByteBuffer {
        val view = csd.duplicate()
        if (view.remaining() >= 4 && view.get(view.position()) == 0x81.toByte()) view.position(view.position() + 4)
        return view
    }

    // --- one key access unit -------------------------------------------------------------------------------------

    /** Whether [unit] is a key frame of [codec]: H.264 holds an IDR slice, HEVC an IDR picture, AV1 a `KEY_FRAME` OBU. */
    fun isKeyUnit(codec: String, unit: ByteBuffer): Boolean = when (codec) {
        H264 -> nalHeaders(unit).any { it and 31 == 5 }
        HEVC -> hevcTypes(unit).any { it == HEVC_IDR_W_RADL || it == HEVC_IDR_N_LP }
        AV1 -> av1IsKeyUnit(unit)
        else -> false
    }

    /**
     * Whether [headers] (stored parameter sets when [spliceable], or the bare key unit) carries what a picture group
     * must start with. Stored AV1 configuration OBUs are spliced before the key frame, so every OBU in them must carry
     * `obu_has_size_field` (AV1 5.2) or the last one would swallow the following frame OBU.
     */
    fun hasParameterSets(codec: String, headers: ByteBuffer, spliceable: Boolean = false): Boolean = when (codec) {
        HEVC -> hevcHasParameterSets(headers)
        AV1 -> av1Obus(headers)?.let { obus ->
            (!spliceable || obus.all { it.hasSize }) && obus.any { it.type == OBU_SEQUENCE_HEADER }
        } == true
        else -> true
    }

    /**
     * Where the stored parameter sets go into a key access unit [unit]: the byte offset, or `null` when the unit
     * has them already. H.264 gets them in front always (as before BK7b), HEVC when the unit lacks any of VPS, SPS
     * and PPS, AV1 right behind the temporal delimiter when it lacks the sequence header.
     */
    fun configOffset(codec: String, unit: ByteBuffer): Int? = when (codec) {
        HEVC -> if (hevcHasParameterSets(unit)) null else 0
        AV1 -> if (av1HasSequenceHeader(unit)) null else av1AfterTemporalDelimiter(unit)
        else -> 0
    }
}
