package app.smartexplorer.android.ui.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.width
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp

/** A form shares its measured label width. Large text and narrow screens stack the same fields. */
@Composable
internal fun SettingsValue(label: String, value: String, labels: List<String>) {
    val measure = rememberTextMeasurer()
    val style = MaterialTheme.typography.bodyMedium
    val labelStyle = MaterialTheme.typography.labelLarge
    val density = LocalDensity.current
    val labelWidth = with(density) { labels.maxOfOrNull { measure.measure(AnnotatedString(it), style = labelStyle).size.width }?.toDp() ?: 0.dp }
    val minimumValue = with(density) { (style.fontSize.toPx() * 15).toDp() }
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        if (maxWidth < labelWidth + minimumValue + 16.dp) {
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(label, style = MaterialTheme.typography.labelLarge)
                Text(value, style = style)
            }
        } else {
            Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                Text(label, modifier = Modifier.width(labelWidth), style = MaterialTheme.typography.labelLarge)
                Text(value, modifier = Modifier.weight(1f), style = style)
            }
        }
    }
}

/** A stable status position for a real request or mutation; errors remain until retried. */
@Composable
internal fun SettingsStatusRow(loading: Boolean = false, saving: Boolean = false, message: String? = null, error: String? = null) {
    Column(Modifier.fillMaxWidth().semantics { liveRegion = LiveRegionMode.Polite }) {
        if (loading || saving) {
            LinearProgressIndicator(Modifier.fillMaxWidth())
            Text(if (saving) "Wird gespeichert …" else "Wird geladen …")
        }
        error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        if (error == null) message?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
    }
}
