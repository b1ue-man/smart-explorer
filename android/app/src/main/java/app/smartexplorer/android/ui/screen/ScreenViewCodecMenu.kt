package app.smartexplorer.android.ui.screen

import androidx.compose.foundation.layout.Column
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

/** One entry of a choice list: disabled with its German [reason] as a second line when this pair cannot run it. */
@Composable
private fun ChoiceItem(label: String, selected: Boolean, reason: String?, onClick: () -> Unit) {
    DropdownMenuItem(
        text = {
            Column {
                Text(label)
                if (reason != null) Text(reason, style = MaterialTheme.typography.bodySmall)
            }
        },
        leadingIcon = { RadioButton(selected = selected, onClick = null, enabled = reason == null) },
        enabled = reason == null,
        onClick = onClick,
    )
}

/**
 * The entries „Codec“ (Automatisch, H.264, HEVC, AV1, Scharf (GPU), Verlustfrei), „Dekodierer“ (Automatisch, Hardware, Software)
 * and „Höchste Datenrate“ of the settings menu (plan 2026-11-14 `bildkette.md` 4.10). A change goes to [onSettings];
 * the data rate opens [MaxRateDialog] through [onRate]. User rule of 2026-11-24 („was du anzeigst MUSS funktionieren“):
 * an entry this phone and the host of [ui] cannot run together is disabled with its reason ([ScreenCodecOffer]), and a
 * stored choice the session runs as „Automatisch“ is named with its reason ([ScreenSessionState.sessionNotes]).
 */
@Composable
internal fun ColumnScope.CodecMenuItems(settings: ViewSettings, onSettings: (ViewSettings) -> Unit, onRate: () -> Unit, ui: ScreenSessionState) {
    val choice = settings.choice
    val backend = LocalScreenBackend.current
    // The phone's decoders are read once per choice and host (MediaCodecList), not at every recomposition.
    val (codecReasons, decoderReasons) = remember(choice, ui.hostCodecs, backend) {
        val offersFor = { wanted: ViewerCodecChoice -> backend.offers(wanted) }
        val running = ScreenCodecOffer.sessionChoice(choice, ui.hostCodecs, offersFor).choice
        ScreenCodecOffer.codecReasons(choice, ui.hostCodecs, offersFor) to
            ScreenCodecOffer.decoderReasons(running, ui.hostCodecs, offersFor)
    }
    Text(
        stringResource(R.string.screen_view_codec),
        style = MaterialTheme.typography.labelMedium,
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
    )
    ViewerCodecChoice.CODECS.forEach { codec ->
        ChoiceItem(stringResource(codecLabelRes(codec)), choice.codec == codec, codecReasons[codec]) {
            onSettings(settings.copy(choice = choice.copy(codec = codec)))
        }
    }
    Text(
        stringResource(R.string.screen_view_decoder),
        style = MaterialTheme.typography.labelMedium,
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
    )
    ViewerCodecChoice.DECODERS.forEach { decoder ->
        ChoiceItem(stringResource(decoderLabelRes(decoder)), choice.decoder == decoder, decoderReasons[decoder]) {
            onSettings(settings.copy(choice = choice.copy(decoder = decoder)))
        }
    }
    ui.sessionNotes.forEach { note ->
        Text(
            note,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.error,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
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
