package app.smartexplorer.android.ui.screen

import app.smartexplorer.android.api.DecoderOffer
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The phone's menu „Codec“ and „Dekodierer“ offers only what works with the host (user rule of 2026-11-24: „was du
 * anzeigst MUSS funktionieren“): host encoders × phone decoders → the reason of every entry that cannot run, and a stored
 * choice the host cannot serve runs as „Automatisch“ for that session only. Pure functions; no MediaCodec runs here.
 */
class ScreenCodecOfferTest {
    /** Hardware H.264 (High, Constrained Baseline), HEVC and AV1 (`main`) and the tiles. */
    private val phone = listOf(
        DecoderOffer(codec = "h264", profile = "high", hardware = true),
        DecoderOffer(codec = "h264", profile = "cb", hardware = true),
        DecoderOffer(codec = "hevc", profile = "main", hardware = true),
        DecoderOffer(codec = "av1", profile = "main", hardware = true),
        DecoderOffer(codec = "tiles"),
    )
    private val offersFor = { _: ViewerCodecChoice -> phone }

    /** A Windows host with an NVIDIA RTX A4500: NVENC H.264 and HEVC (4:2:0 and 4:4:4), no AV1 encoder. */
    private val nvidia = listOf(
        HostCodec("h264", "high", hardware = true),
        HostCodec("hevc", null, hardware = true),
        HostCodec("hevc", "high", hardware = true),
        HostCodec("tiles"),
    )

    /** A Linux host without GPU and without OpenH264: the tiles only. */
    private val tilesOnly = listOf(HostCodec("tiles"))

    private val noAv1 = "dieser Gastgeber hat keinen AV1-Kodierer"

    @Test
    fun codec_offer_task_android_codec_entries_follow_host_encoders_and_phone_decoders() {
        val auto = ViewerCodecChoice()
        assertEquals(
            mapOf("auto" to null, "h264" to null, "hevc" to null, "av1" to noAv1, "sharp" to null, "lossless" to null),
            ScreenCodecOffer.codecReasons(auto, nvidia, offersFor),
        )
        val noGpu = "dieser Gastgeber hat keinen GPU-Kodierer für H.264 oder HEVC"
        assertEquals(
            mapOf(
                "auto" to null,
                "h264" to "dieser Gastgeber hat keinen H.264-Kodierer",
                "hevc" to "dieser Gastgeber hat keinen HEVC-Kodierer",
                "av1" to noAv1,
                "sharp" to noGpu,
                "lossless" to null,
            ),
            ScreenCodecOffer.codecReasons(auto, tilesOnly, offersFor),
        )
        // OpenH264 (Constrained Baseline, software) pairs with the phone's H.264 decoders, „Scharf (GPU)“ still not.
        val openH264 = listOf(HostCodec("h264", "cb", hardware = false), HostCodec("tiles"))
        val reasons = ScreenCodecOffer.codecReasons(auto, openH264, offersFor)
        assertNull(reasons["h264"])
        assertEquals(noGpu, reasons["sharp"])
        // A 4:4:4 encoder alone does not pair with the phone's 4:2:0 decoders.
        val only444 = listOf(HostCodec("hevc", "high", hardware = true))
        assertEquals(
            "kein Kodierer dieses Gastgebers passt zu einem Dekodierer dieses Geräts",
            ScreenCodecOffer.codecReasons(auto, only444, offersFor)["hevc"],
        )
        // Before the host's encoders are known only the tiles' entries are sure.
        val unknown = ScreenCodecOffer.codecReasons(auto, null, offersFor)
        assertNull(unknown["auto"])
        assertNull(unknown["lossless"])
        for (codec in listOf("h264", "hevc", "av1", "sharp")) assertTrue(codec, unknown[codec] != null)
        // A phone without an AV1 decoder, a host with an AV1 encoder.
        val withoutAv1 = { _: ViewerCodecChoice -> phone.filter { it.codec != "av1" } }
        val av1Host = nvidia + HostCodec("av1", null, hardware = true)
        assertEquals(
            "dieses Gerät hat keinen AV1-Dekodierer",
            ScreenCodecOffer.codecReasons(auto, av1Host, withoutAv1)["av1"],
        )
        // „Dekodierer: Hardware“: the tiles decode in software only.
        val hardware = auto.copy(decoder = ViewerCodecChoice.DECODER_HARDWARE)
        assertEquals(
            "die Kacheln dekodiert nur Software (Dekodierer „Hardware“)",
            ScreenCodecOffer.codecReasons(hardware, nvidia, offersFor)["lossless"],
        )
        // „Dekodierer“ under the session's codec: „Hardware“ has no pairing with a host of tiles only.
        val decoders = ScreenCodecOffer.decoderReasons(auto, tilesOnly, offersFor)
        assertNull(decoders["auto"])
        assertNull(decoders["software"])
        assertTrue(decoders["hardware"]!!.startsWith("kein Kodierer dieses Gastgebers passt"))
        // The hello's decoders for „Scharf (GPU)“: H.264 and HEVC alone.
        val sharp = ScreenCodecOffer.namedDecoders("sharp", "auto", phone)
        assertTrue(sharp.all { it.codec == "h264" || it.codec == "hevc" })
    }

    @Test
    fun codec_offer_task_android_stored_choice_the_host_cannot_serve_runs_automatic() {
        val av1 = ViewerCodecChoice(codec = "av1", maxRateBps = 20_000_000L)
        val served = ScreenCodecOffer.sessionChoice(av1, nvidia, offersFor)
        assertEquals("auto", served.choice.codec)
        assertEquals(20_000_000L, served.choice.maxRateBps)
        assertEquals(av1, served.choice.replaces)
        assertEquals(listOf("AV1: $noAv1 – diese Sitzung: Automatisch"), served.notes)
        // The stored choice stays: a new data rate keeps the session, another codec opens it anew.
        assertFalse(ViewerCodecChoice.needsReopen(served.choice, av1.copy(maxRateBps = 8_000_000L)))
        assertTrue(ViewerCodecChoice.needsReopen(served.choice, av1.copy(codec = "h264")))
        // What the host serves runs as stored.
        for (codec in listOf("auto", "h264", "hevc", "sharp", "lossless")) {
            val stored = ViewerCodecChoice(codec = codec)
            val kept = ScreenCodecOffer.sessionChoice(stored, nvidia, offersFor)
            assertEquals(codec, stored, kept.choice)
            assertTrue(codec, kept.notes.isEmpty())
        }
        // An unknown host: the stored choice goes out; it depends on the host.
        assertEquals(av1, ScreenCodecOffer.sessionChoice(av1, null, offersFor).choice)
        assertTrue(ScreenCodecOffer.hostMatters(av1, offersFor))
        assertFalse(ScreenCodecOffer.hostMatters(ViewerCodecChoice(), offersFor))
        assertFalse(ScreenCodecOffer.hostMatters(ViewerCodecChoice(codec = "lossless"), offersFor))
        // A host of tiles only and „Dekodierer: Hardware“: codec and stage give way, each named.
        val hardware = ViewerCodecChoice(codec = "h264", decoder = ViewerCodecChoice.DECODER_HARDWARE)
        assertTrue(ScreenCodecOffer.hostMatters(hardware.copy(codec = "auto"), offersFor))
        val relaxed = ScreenCodecOffer.sessionChoice(hardware, tilesOnly, offersFor)
        assertEquals("auto", relaxed.choice.codec)
        assertEquals(ViewerCodecChoice.DECODER_AUTO, relaxed.choice.decoder)
        assertEquals(2, relaxed.notes.size)
        assertTrue(relaxed.notes[1].startsWith("Dekodierer „Hardware“: kein Kodierer"))
    }

    @Test
    fun codec_offer_task_android_host_codecs_come_from_accepted_and_probe() {
        val accepted = Json.parseToJsonElement(
            """{"encoders":[{"codec":"h264","profile":"high","hardware":true},{"codec":"tiles"}],
               "encodersV2":[{"codec":"hevc","hardware":true}],"platform":"windows"}""",
        ).jsonObject
        assertEquals(
            listOf(HostCodec("h264", "high", true), HostCodec("tiles"), HostCodec("hevc", null, true)),
            ScreenCodecOffer.hostCodecs(accepted, "encoders", "encodersV2"),
        )
        // An older host names no v2 list: its encoders are the legacy ones.
        val older = Json.parseToJsonElement("""{"encoders":[{"codec":"tiles"}]}""").jsonObject
        assertEquals(listOf(HostCodec("tiles")), ScreenCodecOffer.hostCodecs(older, "encoders", "encodersV2"))
        val probe = Json.parseToJsonElement("""{"codecs":[{"codec":"av1","hardware":false}]}""").jsonObject
        assertEquals(listOf(HostCodec("av1")), ScreenCodecOffer.hostCodecs(probe, "codecs"))
        assertNull(ScreenCodecOffer.hostCodecs(probe, "encoders"))
    }
}
