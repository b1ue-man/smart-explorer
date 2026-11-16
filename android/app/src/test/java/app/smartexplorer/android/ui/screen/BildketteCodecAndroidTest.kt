package app.smartexplorer.android.ui.screen

import android.media.MediaCodecInfo.CodecProfileLevel
import app.smartexplorer.android.api.DecoderOffer
import app.smartexplorer.android.screen.ScreenEncoderCodecs
import java.nio.ByteBuffer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * HEVC and AV1 of the phone and the viewer's choices „Codec“, „Dekodierer“ and „Höchste Datenrate“ (plan 2026-11-14
 * `bildkette.md`, package BK7b, B-26, B-39, 4.10). Pure functions only: no MediaCodec runs here.
 */
class BildketteCodecAndroidTest {
    private fun bytes(vararg values: Int): ByteBuffer = ByteBuffer.wrap(ByteArray(values.size) { values[it].toByte() })

    private fun hevcNal(type: Int) = intArrayOf(0, 0, 0, 1, type shl 1, 1, 0x55)

    private val hevcCaps = listOf(
        VideoDecoderCaps("hevc", hardware = true, maxWidth = 3840, maxHeight = 2160),
        VideoDecoderCaps("av1", hardware = false, maxWidth = 4096, maxHeight = 2304),
    )

    @Test
    fun bildkette_task_android_codec_choice_replaces_the_lossless_switch_and_keeps_its_value() {
        // The old switch is the codec „Verlustfrei“ with the same meaning.
        assertFalse(ViewSettings().lossless)
        assertTrue(ViewSettings(choice = ViewerCodecChoice(codec = "lossless")).lossless)
        // A stored `lossless` of the old switch becomes the codec; a stored codec wins over it.
        assertEquals(ViewerCodecChoice(codec = "lossless"), ScreenViewPrefs.decode(null, null, null, legacyLossless = true))
        assertEquals(ViewerCodecChoice(), ScreenViewPrefs.decode(null, null, null, legacyLossless = false))
        assertEquals(
            ViewerCodecChoice("hevc", "software", 20_000_000L),
            ScreenViewPrefs.decode("hevc", "software", 20_000_000L, legacyLossless = true),
        )
        // A name this version does not know, and a rate of 0, are the automatic choice.
        assertEquals(ViewerCodecChoice(), ViewerCodecChoice.of("vp9", "gpu", 0L))
    }

    @Test
    fun bildkette_task_android_only_a_changed_negotiation_opens_a_new_session() {
        val open = ViewerCodecChoice("hevc", "auto", null)
        // Verlustfrei and the data rate go live; the same codec and decoder change nothing in the negotiation.
        assertFalse(ViewerCodecChoice.needsReopen(open, open.copy(codec = "lossless")))
        assertFalse(ViewerCodecChoice.needsReopen(open, open.copy(maxRateBps = 8_000_000L)))
        assertFalse(ViewerCodecChoice.needsReopen(open, open))
        // Another codec or another decoder stage is another negotiation.
        assertTrue(ViewerCodecChoice.needsReopen(open, open.copy(codec = "av1")))
        assertTrue(ViewerCodecChoice.needsReopen(open, open.copy(codec = "auto")))
        assertTrue(ViewerCodecChoice.needsReopen(open, open.copy(decoder = "hardware")))
        // A session opened as Verlustfrei needs a new one to leave it.
        assertTrue(ViewerCodecChoice.needsReopen(ViewerCodecChoice("lossless"), ViewerCodecChoice("h264")))
    }

    @Test
    fun bildkette_task_android_highest_data_rate_is_typed_in_mbit_per_second() {
        assertEquals(20_000_000L, ViewerCodecChoice.parseMbit("20"))
        assertEquals(7_500_000L, ViewerCodecChoice.parseMbit("7,5"))
        assertEquals(7_500_000L, ViewerCodecChoice.parseMbit(" 7.5 "))
        assertNull(ViewerCodecChoice.parseMbit("0"))
        assertNull(ViewerCodecChoice.parseMbit("-3"))
        assertNull(ViewerCodecChoice.parseMbit("schnell"))
        assertNull(ViewerCodecChoice.parseMbit("1e400"))
        assertEquals("20", ViewerCodecChoice.mbitText(20_000_000L))
        assertEquals("7,5", ViewerCodecChoice.mbitText(7_500_000L))
        assertEquals("", ViewerCodecChoice.mbitText(null))
    }

    @Test
    fun bildkette_task_android_software_hevc_and_av1_run_only_by_choice_or_without_hardware() {
        val auto = ViewerCodecChoice()
        // H.264 runs on every stage as before; a hardware decoder of HEVC or AV1 runs.
        assertTrue(ScreenDecoderPolicy.allows(auto, "h264", hardware = false, hardwareExists = true))
        assertTrue(ScreenDecoderPolicy.allows(auto, "hevc", hardware = true, hardwareExists = true))
        // A software decoder of HEVC: not in automatic mode, even without hardware, unless the user named the codec.
        assertFalse(ScreenDecoderPolicy.allows(auto, "hevc", hardware = false, hardwareExists = true))
        assertFalse(ScreenDecoderPolicy.allows(auto, "hevc", hardware = false, hardwareExists = false))
        assertTrue(ScreenDecoderPolicy.allows(auto.copy(codec = "hevc"), "hevc", hardware = false, hardwareExists = false))
        assertFalse(ScreenDecoderPolicy.allows(auto.copy(codec = "hevc"), "hevc", hardware = false, hardwareExists = true))
        // „Dekodierer: Software“ lets the software decoder run; „Hardware“ never does.
        assertTrue(ScreenDecoderPolicy.allows(auto.copy(decoder = "software"), "av1", hardware = false, hardwareExists = true))
        assertFalse(ScreenDecoderPolicy.allows(auto.copy(decoder = "software"), "av1", hardware = true, hardwareExists = true))
        assertFalse(ScreenDecoderPolicy.allows(auto.copy(decoder = "hardware"), "h264", hardware = false, hardwareExists = true))
    }

    @Test
    fun bildkette_task_android_offers_follow_the_choice_and_h264_stays_the_fallback() {
        val h264 = listOf(
            DecoderOffer("h264", "high", true, 4096, 2160),
            DecoderOffer("h264", "cb", false, 1920, 1088),
            DecoderOffer(codec = "tiles", pixelAccess = true),
        )
        fun offers(choice: ViewerCodecChoice) =
            ScreenDecoderPolicy.combine(h264, ScreenDecoderPolicy.videoOffers(hevcCaps, choice), choice)
        // Automatic: H.264, the hardware HEVC decoder (4:2:0 `main`), no software AV1, `tiles` last.
        assertEquals(
            listOf(
                DecoderOffer("h264", "high", true, 4096, 2160),
                DecoderOffer("h264", "cb", false, 1920, 1088),
                DecoderOffer("hevc", "main", true, 3840, 2160),
                DecoderOffer(codec = "tiles", pixelAccess = true),
            ),
            offers(ViewerCodecChoice()),
        )
        // „Codec: AV1“ asks for the software AV1 decoder because the phone has no hardware one; nothing else of HEVC.
        assertEquals(
            listOf(
                DecoderOffer("h264", "high", true, 4096, 2160),
                DecoderOffer("h264", "cb", false, 1920, 1088),
                DecoderOffer("av1", "main", false, 4096, 2304),
                DecoderOffer(codec = "tiles", pixelAccess = true),
            ),
            offers(ViewerCodecChoice(codec = "av1")),
        )
        // „Codec: H.264“ and „Verlustfrei“ offer no HEVC and no AV1.
        assertTrue(offers(ViewerCodecChoice(codec = "h264")).none { it.codec == "hevc" || it.codec == "av1" })
        assertTrue(offers(ViewerCodecChoice(codec = "lossless")).none { it.codec == "hevc" || it.codec == "av1" })
        // „Dekodierer: Hardware“ leaves the hardware decoders and `tiles`, „Software“ the software ones and `tiles`.
        assertEquals(
            listOf(DecoderOffer("h264", "high", true, 4096, 2160), DecoderOffer("hevc", "main", true, 3840, 2160), DecoderOffer(codec = "tiles", pixelAccess = true)),
            offers(ViewerCodecChoice(decoder = "hardware")),
        )
        assertEquals(
            listOf(DecoderOffer("h264", "cb", false, 1920, 1088), DecoderOffer("av1", "main", false, 4096, 2304), DecoderOffer(codec = "tiles", pixelAccess = true)),
            offers(ViewerCodecChoice(decoder = "software")),
        )
        // A phone without HEVC or AV1 decoders offers H.264 and `tiles` as before.
        assertEquals(h264, ScreenDecoderPolicy.combine(h264, ScreenDecoderPolicy.videoOffers(emptyList(), ViewerCodecChoice()), ViewerCodecChoice()))
    }

    @Test
    fun bildkette_task_android_host_names_mime_profile_and_hardware_rule_per_codec() {
        assertEquals("video/avc", ScreenEncoderCodecs.mime("h264"))
        assertEquals("video/hevc", ScreenEncoderCodecs.mime("hevc"))
        assertEquals("video/av01", ScreenEncoderCodecs.mime("av1"))
        assertNull(ScreenEncoderCodecs.mime("vp9"))
        assertEquals(CodecProfileLevel.AVCProfileConstrainedBaseline, ScreenEncoderCodecs.profile("h264", "cb"))
        assertEquals(CodecProfileLevel.AVCProfileHigh, ScreenEncoderCodecs.profile("h264", "high"))
        assertEquals(CodecProfileLevel.HEVCProfileMain, ScreenEncoderCodecs.profile("hevc", "main"))
        assertEquals(CodecProfileLevel.AV1ProfileMain8, ScreenEncoderCodecs.profile("av1", "main"))
        // Android names no 4:4:4 profile of HEVC or AV1: the phone never encodes them.
        assertNull(ScreenEncoderCodecs.profile("hevc", "main_444"))
        assertNull(ScreenEncoderCodecs.profile("av1", "high"))
        // HEVC and AV1 only on a hardware encoder; H.264 on every encoder as before.
        assertFalse(ScreenEncoderCodecs.hardwareOnly("h264"))
        assertTrue(ScreenEncoderCodecs.hardwareOnly("hevc") && ScreenEncoderCodecs.hardwareOnly("av1"))
        assertEquals("high", ScreenEncoderCodecs.offerProfile("h264"))
        assertEquals("main", ScreenEncoderCodecs.offerProfile("hevc"))
    }

    @Test
    fun bildkette_task_android_hevc_key_units_carry_their_parameter_sets() {
        val vps = hevcNal(32); val sps = hevcNal(33); val pps = hevcNal(34)
        val idr = hevcNal(19); val delta = hevcNal(1)
        val all = bytes(*(vps + sps + pps + idr))
        assertTrue(ScreenEncoderCodecs.hevcHasParameterSets(all))
        assertEquals(listOf(32, 33, 34, 19), ScreenEncoderCodecs.hevcTypes(all))
        assertTrue(ScreenEncoderCodecs.isKeyUnit("hevc", all))
        assertFalse(ScreenEncoderCodecs.isKeyUnit("hevc", bytes(*delta)))
        // An IDR without the sets gets the stored ones in front; a unit that has them gets nothing.
        assertEquals(0, ScreenEncoderCodecs.configOffset("hevc", bytes(*idr)))
        assertNull(ScreenEncoderCodecs.configOffset("hevc", all))
        assertFalse(ScreenEncoderCodecs.hasParameterSets("hevc", bytes(*(vps + sps + idr))))
        // H.264 keeps its rule: the stored SPS and PPS always in front.
        assertEquals(0, ScreenEncoderCodecs.configOffset("h264", bytes(0, 0, 0, 1, 0x65, 1)))
        assertTrue(ScreenEncoderCodecs.isKeyUnit("h264", bytes(0, 0, 0, 1, 0x65, 1)))
    }

    @Test
    fun bildkette_task_android_av1_sequence_header_goes_behind_the_temporal_delimiter() {
        val delimiter = intArrayOf(0x12, 0x00)
        val sequence = intArrayOf(0x0A, 0x03, 1, 2, 3)
        val frame = intArrayOf(0x32, 0x02, 9, 9)
        val withoutHeader = bytes(*(delimiter + frame))
        assertFalse(ScreenEncoderCodecs.av1HasSequenceHeader(withoutHeader))
        // The unit lacks the sequence header: it goes right behind the delimiter (2 bytes), or in front without one.
        assertEquals(2, ScreenEncoderCodecs.configOffset("av1", withoutHeader))
        assertEquals(0, ScreenEncoderCodecs.configOffset("av1", bytes(*frame)))
        val complete = bytes(*(delimiter + sequence + frame))
        assertTrue(ScreenEncoderCodecs.av1HasSequenceHeader(complete))
        assertNull(ScreenEncoderCodecs.configOffset("av1", complete))
        assertEquals(listOf(2, 1, 6), ScreenEncoderCodecs.av1Obus(complete)?.map { it.type })
        // The encoder's `av1C` record loses its 4 bytes; bare OBUs stay.
        val record = bytes(*(intArrayOf(0x81, 0x00, 0x0C, 0x00) + sequence))
        assertEquals(sequence.size, ScreenEncoderCodecs.av1ConfigObus(record).remaining())
        assertEquals(sequence.size, ScreenEncoderCodecs.av1ConfigObus(bytes(*sequence)).remaining())
        // A record without configuration OBUs (they are optional) leaves nothing to store; it is no stream of its own.
        assertEquals(0, ScreenEncoderCodecs.av1ConfigObus(bytes(0x81, 0x00, 0x0C, 0x00)).remaining())
        assertFalse(ScreenEncoderCodecs.av1HasSequenceHeader(bytes(0x81, 0x00, 0x0C, 0x00)))
        // Keyframe check (`KEY_FRAME` vs. `INTER_FRAME`), 8-byte padded `leb128` (AV1 4.10.5) and spliceable `csd` OBUs.
        assertTrue(ScreenEncoderCodecs.isKeyUnit("av1", complete))
        assertFalse(ScreenEncoderCodecs.isKeyUnit("av1", bytes(*(delimiter + sequence + intArrayOf(0x32, 0x02, 0x29, 9)))))
        assertTrue(ScreenEncoderCodecs.av1HasSequenceHeader(bytes(0x0A, 0x83, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00, 1, 2, 3)))
        assertNull(ScreenEncoderCodecs.av1Obus(bytes(0x0A, 0x83, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00, 1, 2, 3)))
        assertFalse(ScreenEncoderCodecs.hasParameterSets("av1", bytes(0x08, 1, 2, 3), spliceable = true))
        // A size that runs past the unit is no stream.
        assertNull(ScreenEncoderCodecs.av1Obus(bytes(0x0A, 0x7F, 1)))
    }

    @Test
    fun bildkette_task_android_rate_note_tells_what_the_host_does_with_the_limit() {
        assertEquals(RateNote.Automatic, ScreenChainInfo.rateNote("automatic", null, null))
        assertEquals(RateNote.Applied(8_000_000L), ScreenChainInfo.rateNote("applied", 8_000_000L, 8_000_000L))
        // The host without `rate_ceiling_v1` ignores the limit; the view says so.
        assertEquals(RateNote.Unsupported(8_000_000L), ScreenChainInfo.rateNote("unsupported_by_host", 8_000_000L, 8_000_000L))
        // A limit changed during the session is sent; whether the host knows it stays open until the next connection.
        assertEquals(RateNote.Sent(5_000_000L), ScreenChainInfo.rateNote("automatic", null, 5_000_000L))
        assertEquals(RateNote.Sent(5_000_000L), ScreenChainInfo.rateNote("applied", 8_000_000L, 5_000_000L))
        assertEquals(RateNote.Automatic, ScreenChainInfo.rateNote("applied", 8_000_000L, null))
    }
}
