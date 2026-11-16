package app.smartexplorer.android.ui.screen

import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import app.smartexplorer.android.R

/** The MediaCodec decoder that runs the current picture group: the group's [codec] key, the decoder's [name] and its stage. */
data class DecoderStage(val codec: String, val name: String, val hardware: Boolean)

/**
 * The info line of the full view (plan 2026-11-14 `bildkette.md` 4.10, BK7b): the codec of the stream, the decoder stage of
 * this phone (Hardware or Software, with the decoder's name) and what the host says to „Höchste Datenrate“. Every
 * part is a string resource; [rate] says whether the host knows the limit.
 */
@Composable
internal fun chainInfoLine(codec: String?, stage: DecoderStage?, rate: RateNote): String {
    val parts = ArrayList<String>()
    if (codec != null) {
        // The tile codec is the lossless picture.
        val key = if (codec == "tiles") ViewerCodecChoice.CODEC_LOSSLESS else codec
        parts.add(stringResource(R.string.screen_view_info_codec, stringResource(codecLabelRes(key))))
        if (stage != null && stage.codec == codec) {
            parts.add(
                stringResource(
                    if (stage.hardware) R.string.screen_view_info_decoder_hardware else R.string.screen_view_info_decoder_software,
                    stage.name,
                ),
            )
        }
    }
    when (rate) {
        RateNote.Automatic -> Unit
        is RateNote.Applied -> parts.add(stringResource(R.string.screen_view_info_rate_applied, ViewerCodecChoice.mbitText(rate.bps)))
        is RateNote.Unsupported -> parts.add(stringResource(R.string.screen_view_info_rate_unsupported, ViewerCodecChoice.mbitText(rate.bps)))
        is RateNote.Sent -> parts.add(stringResource(R.string.screen_view_info_rate_sent, ViewerCodecChoice.mbitText(rate.bps)))
    }
    return parts.joinToString(" · ")
}
