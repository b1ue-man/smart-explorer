package app.smartexplorer.android.ui.screen

import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R

/** The label of a codec choice; „Verlustfrei“ and „Scharf (GPU)“ are the old switches under their old names. */
internal fun codecLabelRes(codec: String): Int = when (codec) {
    ViewerCodecChoice.CODEC_H264 -> R.string.screen_view_codec_h264
    ViewerCodecChoice.CODEC_HEVC -> R.string.screen_view_codec_hevc
    ViewerCodecChoice.CODEC_AV1 -> R.string.screen_view_codec_av1
    ViewerCodecChoice.CODEC_SHARP -> R.string.screen_view_sharpness
    ViewerCodecChoice.CODEC_LOSSLESS -> R.string.screen_view_lossless
    else -> R.string.screen_view_codec_auto
}

internal fun decoderLabelRes(decoder: String): Int = when (decoder) {
    ViewerCodecChoice.DECODER_HARDWARE -> R.string.screen_view_decoder_hardware
    ViewerCodecChoice.DECODER_SOFTWARE -> R.string.screen_view_decoder_software
    else -> R.string.screen_view_decoder_auto
}

/**
 * The entries „Codec“ (Automatisch, H.264, HEVC, AV1, Scharf (GPU), Verlustfrei), „Dekodierer“ (Automatisch, Hardware, Software)
 * and „Höchste Datenrate“ of the settings menu (plan 2026-11-14 `bildkette.md` 4.10). A change goes to [onSettings];
 * the data rate opens [MaxRateDialog] through [onRate].
 */
@Composable
internal fun ColumnScope.CodecMenuItems(settings: ViewSettings, onSettings: (ViewSettings) -> Unit, onRate: () -> Unit) {
    val choice = settings.choice
    Text(
        stringResource(R.string.screen_view_codec),
        style = MaterialTheme.typography.labelMedium,
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
    )
    ViewerCodecChoice.CODECS.forEach { codec ->
        DropdownMenuItem(
            text = { Text(stringResource(codecLabelRes(codec))) },
            leadingIcon = { RadioButton(selected = choice.codec == codec, onClick = null) },
            onClick = { onSettings(settings.copy(choice = choice.copy(codec = codec))) },
        )
    }
    Text(
        stringResource(R.string.screen_view_decoder),
        style = MaterialTheme.typography.labelMedium,
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
    )
    ViewerCodecChoice.DECODERS.forEach { decoder ->
        DropdownMenuItem(
            text = { Text(stringResource(decoderLabelRes(decoder))) },
            leadingIcon = { RadioButton(selected = choice.decoder == decoder, onClick = null) },
            onClick = { onSettings(settings.copy(choice = choice.copy(decoder = decoder))) },
        )
    }
    val value = choice.maxRateBps?.let { stringResource(R.string.screen_view_max_rate_mbit, ViewerCodecChoice.mbitText(it)) }
        ?: stringResource(R.string.screen_view_max_rate_auto)
    DropdownMenuItem(
        text = { Text(stringResource(R.string.screen_view_max_rate_line, value)) },
        onClick = onRate,
    )
}

/** Asks for „Höchste Datenrate“ in Mbit/s; empty is automatic ([onSet] gets `null`), a number is bit/s via [ViewerCodecChoice.parseMbit]. */
@Composable
internal fun MaxRateDialog(current: Long?, onDismiss: () -> Unit, onSet: (Long?) -> Unit) {
    var text by remember { mutableStateOf(ViewerCodecChoice.mbitText(current)) }
    val parsed = if (text.isBlank()) null else ViewerCodecChoice.parseMbit(text)
    val invalid = text.isNotBlank() && parsed == null
    // Below 0,15 Mbit/s the host would raise the rate without a word: the dialog says so instead (user rule 2026-11-18).
    val belowMinimum = invalid && ViewerCodecChoice.belowMinimum(text)
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.screen_view_max_rate)) },
        text = {
            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                singleLine = true,
                label = { Text(stringResource(R.string.screen_view_max_rate_prompt)) },
                isError = invalid,
                supportingText = {
                    if (belowMinimum) Text(stringResource(R.string.screen_view_max_rate_minimum, ViewerCodecChoice.mbitText(ViewerCodecChoice.MIN_RATE_BPS)))
                    else if (invalid) Text(stringResource(R.string.screen_view_max_rate_invalid))
                },
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
            )
        },
        confirmButton = { TextButton(enabled = !invalid, onClick = { onSet(parsed) }) { Text(stringResource(R.string.screen_view_max_rate_set)) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.screen_view_max_rate_cancel)) } },
    )
}
