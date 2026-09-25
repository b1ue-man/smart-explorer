package app.smartexplorer.android.ui.transfers

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon

/** Which tasks count as transfers (sheet, bar) and how their progress reads (spec F8). */
object TransferTasks {
    // Tasks that belong to a page of their own (scan, analysis, index, sign-in, …).
    private val OWN_PAGE_KINDS = setOf("scan", "properties", "index", "analyze", "reclaim", "catchup", "oauth", "exec", "update")

    fun isTransfer(task: TaskInfo): Boolean = task.kind !in OWN_PAGE_KINDS

    /** Aggregated progress of [tasks] in `0..1`, by bytes, else by items; `null` if unknown. */
    fun fraction(tasks: List<TaskInfo>): Float? {
        val totalBytes = tasks.sumOf { it.totalBytes.coerceAtLeast(0) }
        if (totalBytes > 0) return Format.fraction(tasks.sumOf { it.doneBytes.coerceAtLeast(0) }, totalBytes)
        val totalItems = tasks.sumOf { it.totalItems.coerceAtLeast(0) }
        return Format.fraction(tasks.sumOf { it.doneItems.coerceAtLeast(0) }, totalItems)
    }

    /** "12 von 40 Dateien · 1,2 GB · 8 MB/s" (parts only when known). */
    fun progressLine(task: TaskInfo): String {
        if (task.state == "queued") return "In Warteschlange"
        val parts = mutableListOf<String>()
        when {
            task.totalItems > 0 -> parts += "${task.doneItems} von ${task.totalItems} Dateien"
            task.doneItems > 0 -> parts += "${task.doneItems} Dateien"
        }
        when {
            task.totalBytes > 0 -> parts += "${Format.size(task.doneBytes)} von ${Format.size(task.totalBytes)}"
            task.doneBytes > 0 -> parts += Format.size(task.doneBytes)
        }
        if (task.rateBps > 0) parts += Format.rate(task.rateBps)
        return parts.joinToString(" · ").ifEmpty { task.message ?: "Läuft…" }
    }

    /** Outcome line of a finished task. */
    fun outcomeLine(task: TaskInfo): String = when (task.state) {
        "done" -> if (task.errors.isEmpty()) "Fertig" else "Fertig · ${task.errors.size} Fehler"
        "canceled" -> task.message?.let { "Abgebrochen: $it" } ?: "Abgebrochen"
        else -> "Fehlgeschlagen" + (task.message?.let { ": $it" } ?: "")
    }
}

/**
 * Bar at the bottom of "Dateien" while transfers run: "2 Übertragungen · 45 % [Anzeigen]".
 * Renders nothing when [tasks] holds no active transfer.
 */
@Composable
fun TransfersBar(tasks: List<TaskInfo>, onShow: () -> Unit, modifier: Modifier = Modifier) {
    val active = tasks.filter { it.isActive && TransferTasks.isTransfer(it) }
    if (active.isEmpty()) return
    val fraction = TransferTasks.fraction(active)
    val label = buildString {
        append(if (active.size == 1) "1 Übertragung" else "${active.size} Übertragungen")
        fraction?.let { append(" · ${(it * 100).toInt()} %") }
    }
    Surface(color = MaterialTheme.colorScheme.surfaceContainerHigh, modifier = modifier.fillMaxWidth()) {
        Column {
            LoadingBar(loading = true, fraction = fraction)
            Row(
                modifier = Modifier.padding(start = 16.dp, end = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                SeIcon(R.drawable.ic_sync, contentDescription = null)
                Text(label, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
                TextButton(onClick = onShow) { Text("Anzeigen") }
            }
        }
    }
}
