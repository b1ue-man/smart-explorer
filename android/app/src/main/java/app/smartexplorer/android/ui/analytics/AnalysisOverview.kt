package app.smartexplorer.android.ui.analytics

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.material3.FilterChip
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.AnalyzeLargestFile
import app.smartexplorer.android.api.AnalyzeOverview
import app.smartexplorer.android.api.AnalyzeTypeTotal
import app.smartexplorer.android.ui.common.Format

/** Views of an analysis result, as on the desktop. */
internal enum class AnalysisTab(val label: String) {
    Treemap("Kacheln"),
    Largest("Größte Dateien"),
    Types("Nach Dateityp"),
}

/** "Größe 5,0 GB · Belegt 100 MB" of one largest file. */
internal fun largestDetails(file: AnalyzeLargestFile, known: Boolean): String = sizesText(file.size, file.allocated, known)

/** "12 Dateien · Größe … · Belegt …" of one extension. */
internal fun typeDetails(total: AnalyzeTypeTotal, known: Boolean): String =
    "${total.files} Dateien · ${sizesText(total.size, total.allocated, known)}"

/** Tab chips "Kacheln | Größte Dateien | Nach Dateityp". */
@Composable
internal fun AnalysisTabs(shown: AnalysisTab, onChoose: (AnalysisTab) -> Unit) {
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        AnalysisTab.entries.forEach { tab ->
            FilterChip(selected = shown == tab, onClick = { onChoose(tab) }, label = { Text(tab.label) })
        }
    }
}

/** Hint above the list: scope, order and what the core folded into aggregates. */
@Composable
internal fun OverviewHint(overview: AnalyzeOverview?) {
    val text = when (overview) {
        null -> "Übersicht wird geladen …"
        else -> buildString {
            append("Angezeigter Ordner mit Unterordnern, sortiert nach ")
            append(if (overview.sort == "allocated") "Belegt" else "Größe").append('.')
            overview.aggregatedNote?.let { append(' ').append(it) }
        }
    }
    Text(text, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
}

/** One row of "Größte Dateien"; tapping opens the file's folder in "Dateien". */
@Composable
internal fun LargestRow(file: AnalyzeLargestFile, known: Boolean, onOpen: (String) -> Unit) {
    val location = file.location
    ListItem(
        headlineContent = { Text(file.path, maxLines = 2, overflow = TextOverflow.Ellipsis) },
        supportingContent = { Text(largestDetails(file, known)) },
        modifier = Modifier.clickable(enabled = location != null) { location?.let(onOpen) },
    )
}

/** One row of "Nach Dateityp". */
@Composable
internal fun TypeRow(total: AnalyzeTypeTotal, known: Boolean) {
    ListItem(
        headlineContent = { Text(total.extension) },
        supportingContent = { Text(typeDetails(total, known)) },
        trailingContent = { Text(Format.size(if (known) total.allocated else total.size)) },
    )
}
