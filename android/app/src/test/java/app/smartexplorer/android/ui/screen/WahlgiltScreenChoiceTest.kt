package app.smartexplorer.android.ui.screen

import app.smartexplorer.android.api.DecoderOffer
import app.smartexplorer.android.api.ScreenDevice
import app.smartexplorer.android.core.ScreenViewEvent
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * User rule of 2026-11-18: every setting the user chooses is used exactly as chosen, an automatic change only under
 * „Automatisch“, a choice that cannot run is a visible error with its reason („Scharf (GPU)“, „Dekodierer“, the
 * lossless fallback, the host's codec reports, „Höchste Datenrate“, „Monitor“/„Auflösung“).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = android.app.Application::class)
class WahlgiltScreenChoiceTest {
    private val hevcAndAv1 = listOf(
        VideoDecoderCaps("hevc", hardware = true, maxWidth = 3840, maxHeight = 2160),
        VideoDecoderCaps("av1", hardware = true, maxWidth = 3840, maxHeight = 2160),
    )

    @Test
    fun wahlgilt_sharp_is_a_stored_codec_and_sets_the_sharpness_bit() {
        assertTrue(ViewerCodecChoice.CODEC_SHARP in ViewerCodecChoice.CODECS)
        // Stored and loaded as chosen, never turned into „Automatisch“.
        assertEquals(ViewerCodecChoice(codec = "sharp"), ScreenViewPrefs.decode("sharp", null, null, legacyLossless = false))
        // The `SET_MODE` bit `sharpness` comes only from the codec „Scharf (GPU)“; „Verlustfrei“ keeps its own bit.
        val sharp = ViewSettings(choice = ViewerCodecChoice(codec = "sharp"))
        assertTrue(sharp.sharpness)
        assertFalse(sharp.lossless)
        assertFalse(ViewSettings().sharpness)
        assertFalse(ViewSettings(choice = ViewerCodecChoice(codec = "lossless")).sharpness)
        // A change to or from „Scharf (GPU)“ is another negotiation.
        assertTrue(ViewerCodecChoice.needsReopen(ViewerCodecChoice(), ViewerCodecChoice(codec = "sharp")))
        // „Scharf (GPU)“ offers HEVC, never AV1.
        val video = ScreenDecoderPolicy.videoOffers(hevcAndAv1, ViewerCodecChoice(codec = "sharp"))
        assertEquals(listOf("hevc"), video.map { it.codec })
    }

    @Test
    fun wahlgilt_hybrid_decoder_keeps_the_chosen_stage() {
        val decoders = listOf("c2.hw.avc" to true, "c2.android.avc" to false)
        assertEquals(listOf("c2.hw.avc"), ScreenDecoderPolicy.inStage("hardware", decoders) { it.second }.map { it.first })
        assertEquals(listOf("c2.android.avc"), ScreenDecoderPolicy.inStage("software", decoders) { it.second }.map { it.first })
        assertEquals(decoders, ScreenDecoderPolicy.inStage("auto", decoders) { it.second })
        // No decoder of the chosen stage: none stands in.
        assertTrue(ScreenDecoderPolicy.inStage("hardware", listOf("c2.android.avc" to false)) { it.second }.isEmpty())
    }

    @Test
    fun wahlgilt_host_codec_reports_show_chosen_against_running() {
        // Another encoder of the same codec: a notice, no error.
        val same = ScreenCodecNotices.parse("encoder_fallback_to_h264: h264 hardware gave way to h264 software: busy", "h264")
        assertNotNull(same)
        assertFalse(same!!.error)
        assertEquals("h264", same.running)
        assertTrue(same.text.contains("gewählt: H.264") && same.text.contains("läuft: H.264"))
        // Under „Automatisch“ every fallback is a notice, also to HEVC or the tiles.
        assertFalse(ScreenCodecNotices.parse("encoder_fallback_to_hevc: av1 hardware gave way to hevc hardware: x", "auto")!!.error)
        assertFalse(ScreenCodecNotices.parse("encoder_fallback_to_tiles: x", "auto")!!.error)
        // A named codec the host left is an error naming both.
        val left = ScreenCodecNotices.parse("encoder_fallback_to_tiles: encoder gone", "hevc")!!
        assertTrue(left.error)
        assertTrue(left.text.contains("HEVC") && left.text.contains("Verlustfrei") && left.text.contains("encoder gone"))
        // „Scharf (GPU)“ may run on H.264 or HEVC.
        assertFalse(ScreenCodecNotices.parse("encoder_fallback_to_hevc: x", "sharp")!!.error)
        // The chosen codec cannot run: an error, with and without the codec named in the report.
        val unavailable = ScreenCodecNotices.parse("chosen_codec_unavailable: av1: the host has no encoder of this codec", "av1")!!
        assertTrue(unavailable.error)
        assertTrue(unavailable.text.contains("AV1") && unavailable.text.contains("keinen Kodierer"))
        val bare = ScreenCodecNotices.parse("chosen_codec_unavailable: no hardware encoder", "hevc")!!
        assertTrue(bare.error && bare.text.contains("HEVC") && bare.text.contains("no hardware encoder"))
        assertTrue(ScreenCodecNotices.parse("chosen_codec_failed: h264: no encoder of the chosen codec is left", "h264")!!.error)
        assertNull(ScreenCodecNotices.parse("pipewire_missing", "h264"))
        // The session state keeps the notice and the error apart.
        val named = ScreenSessionState(chosenCodec = "h264").apply(ScreenViewEvent.State(7, "streaming", "encoder_fallback_to_tiles: x"))
        assertNotNull(named.codecError)
        assertNull(named.codecFallback)
        val automatic = ScreenSessionState().apply(ScreenViewEvent.State(7, "streaming", "encoder_fallback_to_tiles: x"))
        assertNull(automatic.codecError)
        assertNotNull(automatic.codecFallback)
        val ended = ScreenSessionState(chosenCodec = "av1").apply(ScreenViewEvent.Ended(7, "encoder_failed", "chosen_codec_unavailable: av1: x"))
        assertNotNull(ended.codecError)
    }

    /** Review 2026-11-19 (T1): the host's new reasons read German on the phone as on the desktop. */
    @Test
    fun review1119_task_new_host_reasons_read_german() {
        val sharp = ScreenCodecNotices.parse(
            "chosen_codec_failed: sharp: no encoder of the chosen codec is left: Scharf (GPU): no GPU encoder of hevc " +
                "is left (failed: [nvenc-hevc]; software passed over: none)", "sharp")!!
        assertTrue(sharp.text, sharp.text.contains("Scharf (GPU): kein GPU-Kodierer für HEVC mehr übrig (ausgefallen: [nvenc-hevc])"))
        val lowered = ScreenCodecNotices.parse(
            "encoder_fallback_to_h264: h264 hardware gave way to h264 software: the encoder refused to lower its bit rate " +
                "from 6000000 to 3000000 bit/s", "h264")!!
        assertTrue(lowered.text, lowered.text.contains("der Kodierer lehnte es ab, seine Bitrate von 6000000 auf 3000000 bit/s zu senken"))
        assertEquals(
            "OpenH264 lehnte es ab, seine Bitrate von 6000000 auf 3000000 bit/s zu senken",
            ScreenCodecNotices.knownError("OpenH264 refused to lower its bit rate from 6000000 to 3000000 bit/s"),
        )
        assertEquals(
            "nvenc-h264 lehnte es ab, seine Bitrate auf 3000000 bit/s zu senken; Meldung: status 8",
            ScreenCodecNotices.knownError("nvenc-h264 refused to lower its bit rate to 3000000 bit/s: status 8"),
        )
        val small = ScreenCodecNotices.parse(
            "chosen_codec_unavailable: h264: the picture of 1x1 pixels is smaller than the 2x2 pixels the codec needs; " +
                "the stream waits for a larger one", "h264")!!
        assertTrue(small.error)
        assertTrue(small.text, small.text.contains("H.264 gewählt, das Bild mit 1x1 Pixeln ist kleiner als die 2×2 Pixel"))
        assertNull(ScreenCodecNotices.knownError("device lost"))
        // A notice of the host's encoder is no error and reads German.
        val notice = ScreenCodecNotices.parse(
            "encoder_notice: the encoder's lowest bit rate 250000 bit/s is above the highest data rate 150000 bit/s; " +
                "fewer pictures keep the ceiling", "h264")!!
        assertFalse(notice.error)
        assertEquals(
            "Hinweis vom Gerät: Die niedrigste Bitrate des Kodierers (250000 bit/s) liegt über der Höchsten Datenrate " +
                "(150000 bit/s); weniger Bilder halten die Obergrenze ein",
            notice.text,
        )
        assertEquals(
            "Hinweis vom Gerät: Der Kodierer lehnte den Parameter bitrate-mode ab; Meldung: CBR",
            ScreenCodecNotices.noticeText("the encoder refused the parameter bitrate-mode: CBR"),
        )
    }

    @Test
    fun wahlgilt_highest_data_rate_has_the_hosts_floor() {
        assertEquals(150_000L, ViewerCodecChoice.MIN_RATE_BPS)
        assertEquals(150_000L, ViewerCodecChoice.parseMbit("0,15"))
        assertNull(ViewerCodecChoice.parseMbit("0,1"))
        assertTrue(ViewerCodecChoice.belowMinimum("0,1"))
        assertFalse(ViewerCodecChoice.belowMinimum("0,15"))
        assertFalse(ViewerCodecChoice.belowMinimum("schnell"))
        assertFalse(ViewerCodecChoice.belowMinimum("0"))
        assertEquals("0,15", ViewerCodecChoice.mbitText(ViewerCodecChoice.MIN_RATE_BPS))
    }

    @Test
    fun wahlgilt_display_state_line_names_chosen_actual_and_reason() {
        val chosen = ViewerDisplayChoice(wish = "request", width = 2560, height = 1440)
        val unsupported = buildJsonObject { put("state", "unsupported"); put("driver", "windows"); put("reason", "driver_missing") }
        val line = ScreenDisplayReportText.line(chosen, unsupported, null)
        assertTrue(line, line.contains("eigener (virtuell)") && line.contains("2560×1440"))
        assertTrue(line, line.contains("nicht möglich (driver_missing)"))
        // Without a report the host does not support the choice.
        assertTrue(ScreenDisplayReportText.line(ViewerDisplayChoice(), null, null).endsWith("vom Gerät nicht unterstützt"))
        // The statistics name the display now when it differs from the report.
        val active = buildJsonObject { put("state", "active"); put("width", 1920); put("height", 1080); put("driver", "randr") }
        assertEquals("active:1920x1080:randr", ScreenDisplayReportText.chainKey(active))
        assertFalse(ScreenDisplayReportText.line(chosen, active, "active:1920x1080:randr").contains("jetzt"))
        assertTrue(ScreenDisplayReportText.line(chosen, active, "inactive:off").contains("jetzt inactive:off"))
        // The host set another size than the chosen one: chosen, set and the reason, also in the statistics key.
        val adjusted = buildJsonObject {
            put("state", "active"); put("width", 1920); put("height", 1080); put("driver", "windows")
            putJsonObject("requested") { put("width", 2560); put("height", 1440) }
            put("adjusted", "refused: mode 2560x1440 not listed")
            put("source", "monitor:2"); put("sourceChosen", "monitor:1")
        }
        val text = ScreenDisplayReportText.reportText(adjusted)
        assertTrue(text, text.contains("gewählt 2560×1440, eingestellt 1920×1080, Grund: der Monitor lehnte den Modus ab: mode 2560x1440 not listed"))
        assertTrue(text, text.contains("Quelle: monitor:1 → jetzt monitor:2"))
        assertEquals("active:1920x1080:windows;requested=2560x1440;adjusted=refused;source=monitor:2", ScreenDisplayReportText.chainKey(adjusted))
        assertFalse(ScreenDisplayReportText.line(chosen, adjusted, "active:1920x1080:windows;requested=2560x1440;adjusted=refused;source=monitor:2").contains("jetzt active"))
        // A window source: the monitor wish applies to monitor sources only; `Inactive` names its size.
        val window = buildJsonObject {
            put("state", "unsupported"); put("reason", "source_not_monitor")
            putJsonObject("requested") { put("width", 2560); put("height", 1440) }; put("adjusted", "source_not_monitor")
        }
        assertTrue(ScreenDisplayReportText.reportText(window).contains("Monitor-Wunsch gilt nur für Monitor-Quellen"))
        assertTrue(ScreenDisplayReportText.reportText(window).contains("eingestellt keine, Grund: die gewählte Quelle ist kein Monitor"))
        val physical = buildJsonObject { put("state", "inactive"); put("reason", "physical_mode_refused"); put("width", 1920); put("height", 1080) }
        assertTrue(ScreenDisplayReportText.reportText(physical).contains("lehnte die gewählte Auflösung ab, 1920×1080"))
        // The `accepted` event and the statistics fill the state.
        val state = ScreenSessionState()
            .apply(ScreenViewEvent.Accepted(7, "direct", "linux", "allow", "allow", buildJsonObject { put("virtualDisplay", active) }))
            .apply(ScreenViewEvent.Stats(7, buildJsonObject { putJsonObject("chain") { put("virtualDisplay", "inactive:off") } }))
        assertEquals(active, state.virtualDisplay)
        assertEquals("inactive:off", state.chainVirtualDisplay)
    }

    @Test
    fun wahlgilt_tiles_start_reopens_for_a_video_codec() {
        assertTrue(ViewerCodecChoice.reopenForVideo("tiles", ViewerCodecChoice(codec = "hevc")))
        assertTrue(ViewerCodecChoice.reopenForVideo("tiles", ViewerCodecChoice(codec = "sharp")))
        assertFalse(ViewerCodecChoice.reopenForVideo("tiles", ViewerCodecChoice(codec = "lossless")))
        assertFalse(ViewerCodecChoice.reopenForVideo("h264", ViewerCodecChoice(codec = "hevc")))
        val first = ScreenViewEvent.Stream(7, "tiles", null, 64, 64, 1, "fixture", false)
        val state = ScreenSessionState().apply(first).apply(first.copy(codec = "h264", gop = 2))
        assertEquals("tiles", state.firstCodec)
    }

    private class Backend : ScreenBackend {
        val flow = MutableSharedFlow<ScreenViewEvent>(extraBufferCapacity = 32)
        override val events: Flow<ScreenViewEvent> = flow
        /** `lossless` of every `SET_MODE` that left the app. */
        val lossless = CopyOnWriteArrayList<Boolean>()
        override fun offers() = emptyList<DecoderOffer>()
        override fun pictures(session: Long) = PictureSource { _, _ -> -1 }
        override suspend fun devices(roomId: String?) = emptyList<ScreenDevice>()
        override suspend fun probe(target: ScreenTarget) = error("unused probe")
        override suspend fun open(target: ScreenTarget, mode: String, source: String?, displayWidth: Int, displayHeight: Int, maxFps: Int, decoders: List<DecoderOffer>): Long {
            flow.emit(ScreenViewEvent.Accepted(7, "direct", "windows", "allow", "allow", JsonObject(emptyMap())))
            flow.emit(ScreenViewEvent.Stream(7, "h264", "high", 64, 64, 5, "fixture", true))
            return 7
        }
        override suspend fun setMode(session: Long, mode: String, source: String?, width: Int?, height: Int?, fps: Int?, sharpness: Boolean, lossless: Boolean) {
            this.lossless += lossless
        }
        override suspend fun input(session: Long, events: List<JsonObject>) = events.size
        override suspend fun control(session: Long, action: String) = Unit
        override suspend fun listWindows(session: Long) = Unit
        override suspend fun keyframe(session: Long) = Unit
        override suspend fun close(session: Long) = Unit
    }

    private suspend fun await(condition: () -> Boolean) = withTimeout(5000) {
        while (!condition()) delay(5)
    }

    @Test
    fun wahlgilt_new_codec_choice_ends_the_automatic_lossless_fallback() = runBlocking {
        // „Automatisch“: the host's fallback to the tiles holds until the user chooses another codec.
        val backend = Backend()
        val session = ScreenSession(ScreenTarget("direct", "fixture", null), backend)
        try {
            session.open("full", null, 64, 64, 30, emptyList(), ViewerCodecChoice())
            await { session.state.value.stream != null }
            backend.flow.emit(ScreenViewEvent.State(7, "streaming", "encoder_fallback_to_tiles: encoder gone"))
            await { session.state.value.codecFallback != null }
            session.setMode("full", lossless = false)
            await { backend.lossless.size == 1 }
            assertTrue("the fallback holds under „Automatisch“", backend.lossless[0])
            session.chooseCodec("h264")
            assertNull(session.state.value.codecFallback)
            session.setMode("full", lossless = false)
            await { backend.lossless.size == 2 }
            assertFalse("a new codec choice is not overruled by the fallback before", backend.lossless[1])
        } finally { session.close() }
    }

    @Test
    fun wahlgilt_named_codec_never_falls_back_to_tiles_by_itself() = runBlocking {
        val backend = Backend()
        val session = ScreenSession(ScreenTarget("direct", "fixture", null), backend)
        try {
            session.open("full", null, 64, 64, 30, emptyList(), ViewerCodecChoice(codec = "h264"))
            await { session.state.value.stream != null }
            // The phone cannot decode the picture: an error with the reason, no `SET_MODE` to the tiles.
            session.hybridUnavailable(5, "kein H.264-Dekodierer der gewählten Stufe")
            assertNotNull(session.state.value.codecError)
            assertTrue(session.state.value.codecError!!.contains("H.264"))
            // An older host's tiles fallback under a named codec is an error and forces nothing.
            backend.flow.emit(ScreenViewEvent.State(7, "streaming", "encoder_fallback_to_tiles: encoder gone"))
            await { session.state.value.reason != null }
            session.setMode("full", lossless = false)
            await { backend.lossless.isNotEmpty() }
            assertEquals(listOf(false), backend.lossless.toList())
        } finally { session.close() }
    }
}
