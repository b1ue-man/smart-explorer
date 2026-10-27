package app.smartexplorer.android.ui.screen

import android.media.MediaCodecInfo.CodecProfileLevel
import app.smartexplorer.android.api.DecoderOffer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** The decoders this phone offers the host (SCR-M9, U28-5). */
class ScreenDecoderOffersTest {
    @Test
    fun screen_task_android_decoder_offers_include_tiles() {
        // Without any decoder (and on a machine without MediaCodec) the lossless codec remains.
        // `tiles` always offers CPU pixel access (frame identity, d577bab8).
        assertEquals(listOf(DecoderOffer(codec = "tiles", pixelAccess = true)), ScreenDecoder.offersFor(emptyList()))
        assertTrue(ScreenDecoder.offers().any { it.codec == "tiles" && it.profile == null })
        // The profiles of the hardware and the software decoder are offered apart, `tiles` last.
        val offers = ScreenDecoder.offersFor(
            listOf(
                DecoderCaps(hardware = true, avcProfiles = setOf(CodecProfileLevel.AVCProfileHigh, CodecProfileLevel.AVCProfileBaseline), maxWidth = 4096, maxHeight = 2160),
                DecoderCaps(hardware = false, avcProfiles = setOf(CodecProfileLevel.AVCProfileConstrainedBaseline), maxWidth = 1920, maxHeight = 1088),
            ),
        )
        assertEquals(
            listOf(
                DecoderOffer("h264", "high", true, 4096, 2160),
                DecoderOffer("h264", "cb", true, 4096, 2160),
                DecoderOffer("h264", "cb", false, 1920, 1088),
                DecoderOffer(codec = "tiles", pixelAccess = true),
            ),
            offers,
        )
        assertEquals("tiles", offers.last().codec)
    }
}
