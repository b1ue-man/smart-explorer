package app.smartexplorer.android.ui.analytics

import androidx.annotation.DrawableRes
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import app.smartexplorer.android.R
import app.smartexplorer.android.api.AnalyzeChild
import app.smartexplorer.android.api.AnalyzeKind
import app.smartexplorer.android.system.StorageStatsAccess
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.common.kindIcon

/** Why `Android/data|obb` count as protected rather than unreadable (spec B1). */
internal const val PROTECTED_EXPLANATION =
    "Android erlaubt keiner App, die Ordner anderer Apps unter Android/data und Android/obb zu lesen – " +
        "auch nicht mit „Zugriff auf alle Dateien“. Diese Bereiche zählen daher nicht als Lesefehler."

/** "1 Bereich …" / "n Bereiche von Android geschützt". */
internal fun protectedLabel(count: Long): String =
    if (count == 1L) "1 Bereich von Android geschützt" else "$count Bereiche von Android geschützt"

/** One-line notice with icon and an optional action (read problems, protected areas). */
@Composable
internal fun NoticeRow(
    @DrawableRes icon: Int,
    text: String,
    tint: Color,
    actionLabel: String? = null,
    onAction: (() -> Unit)? = null,
) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        SeIcon(icon, contentDescription = null, tint = tint)
        Text(text, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f).padding(start = 8.dp))
        if (actionLabel != null && onAction != null) TextButton(onClick = onAction) { Text(actionLabel) }
    }
}

/** `true` while usage access is granted; re-checked whenever the UI resumes (return from settings). */
@Composable
internal fun rememberUsageAccess(): Boolean {
    val context = LocalContext.current
    var granted by remember { mutableStateOf(StorageStatsAccess.hasUsageAccess(context)) }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { granted = StorageStatsAccess.hasUsageAccess(context) }
    return granted
}

/**
 * Card "App-Größen anzeigen" (spec B3/B6) for a result that contains `Android/data` of the internal
 * storage: without usage access it leads to the setting (Android 15+: first "Eingeschränkte
 * Einstellungen zulassen" in App-Info); once granted for a result made without it, it offers
 * [Neu analysieren]. Renders nothing when the result already includes Android's figures.
 */
@Composable
internal fun UsageAccessCard(appDataIncluded: Boolean, onReanalyze: () -> Unit, onDismiss: () -> Unit, modifier: Modifier = Modifier) {
    val granted = rememberUsageAccess()
    if (granted && appDataIncluded) return
    val context = LocalContext.current
    Card(
        modifier = modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.secondaryContainer,
            contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
        ),
    ) {
        Row(Modifier.padding(start = 16.dp, end = 16.dp, top = 16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            SeIcon(R.drawable.ic_lock, contentDescription = null)
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                if (granted) {
                    Text("Zugriff auf Nutzungsdaten erteilt", style = MaterialTheme.typography.titleSmall)
                    Text(
                        "Neu analysieren, um den Platz der Apps und ihrer Ordner laut Android zu sehen.",
                        style = MaterialTheme.typography.bodyMedium,
                    )
                } else {
                    Text("App-Größen anzeigen", style = MaterialTheme.typography.titleSmall)
                    Text(
                        "Android sperrt die Ordner anderer Apps unter Android/data. Mit „Zugriff auf Nutzungsdaten“ " +
                            "zeigt die Analyse laut Android, wie viel Platz jede App mit ihren Daten belegt " +
                            "(nur interner Speicher).",
                        style = MaterialTheme.typography.bodyMedium,
                    )
                    if (StorageStatsAccess.restrictedSettingsPossible) {
                        Text(
                            "Lässt sich der Schalter nicht einschalten: App-Info → ⋮ → „Eingeschränkte Einstellungen " +
                                "zulassen“, danach erneut auf Erlauben tippen.",
                            style = MaterialTheme.typography.bodySmall,
                        )
                    }
                }
            }
        }
        Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 4.dp), horizontalArrangement = Arrangement.End) {
            TextButton(onClick = onDismiss) { Text("Ausblenden") }
            if (granted) {
                TextButton(onClick = onReanalyze) { Text("Neu analysieren") }
            } else {
                if (StorageStatsAccess.restrictedSettingsPossible) {
                    TextButton(onClick = {
                        if (!StorageStatsAccess.openAppDetails(context)) Snackbars.show("App-Info nicht verfügbar.")
                    }) { Text("App-Info") }
                }
                TextButton(onClick = {
                    if (!StorageStatsAccess.openUsageAccessSettings(context)) {
                        Snackbars.show(
                            "Einstellung nicht verfügbar – bitte unter Einstellungen → Apps → Spezieller App-Zugriff → " +
                                "Zugriff auf Nutzungsdaten erlauben.",
                        )
                    }
                }) { Text("Erlauben") }
            }
        }
    }
}

/**
 * One row of the folder list: size, share of [total] and a bar; real folders (also protected
 * ones) and the app list open on tap, an app shows its breakdown; estimates ("laut Android") and
 * aggregated rows do nothing.
 */
@Composable
internal fun ChildRow(child: AnalyzeChild, total: Long, onOpen: () -> Unit) {
    val share = shareOf(child.size, total)
    val opens = child.tappable
    ListItem(
        headlineContent = { Text(child.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = {
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(childDetails(child, share))
                LinearProgressIndicator(progress = { share }, modifier = Modifier.fillMaxWidth())
            }
        },
        leadingContent = { SeIcon(childIcon(child), contentDescription = null) },
        trailingContent = if (opens) {
            { SeIcon(R.drawable.ic_chevron_right, contentDescription = null) }
        } else {
            null
        },
        modifier = Modifier.clickable(enabled = opens, onClick = onOpen),
    )
}

private fun childDetails(child: AnalyzeChild, share: Float): String = buildString {
    val kind = child.rowKind
    if (kind == AnalyzeKind.Rest || kind == AnalyzeKind.Apps || kind == AnalyzeKind.App) append("≈ ")
    append(Format.size(child.size)).append(" · ").append((share * 100).toInt()).append(" %")
    when (kind) {
        AnalyzeKind.Dir -> append(" · ").append(child.childCount).append(" Einträge")
        AnalyzeKind.Protected -> {
            if (child.isDir && child.childCount > 0) append(" · ").append(child.childCount).append(" Einträge")
            append(" · von Android geschützt")
        }
        AnalyzeKind.Rest -> append(" · Schätzung laut Android")
        AnalyzeKind.Apps -> append(" · ").append(child.childCount).append(" Apps laut Android")
        AnalyzeKind.App -> append(" · App ").append(Format.size(child.appBytes)).append(" · Daten ").append(Format.size(child.dataBytes))
        AnalyzeKind.File, AnalyzeKind.Aggregate -> Unit
    }
}

@DrawableRes
private fun childIcon(child: AnalyzeChild): Int = when (child.rowKind) {
    AnalyzeKind.Dir -> R.drawable.ic_folder
    AnalyzeKind.Protected -> R.drawable.ic_lock
    AnalyzeKind.Rest -> R.drawable.ic_info
    AnalyzeKind.Apps, AnalyzeKind.App -> R.drawable.ic_apk
    AnalyzeKind.Aggregate -> if (child.isDir) R.drawable.ic_folder else R.drawable.ic_file
    AnalyzeKind.File -> kindIcon(entryKind(tileKind(child)))
}

/** `Entry.kind` values for the file icons (ui.common.kindIcon). */
private fun entryKind(kind: TileKind): String = when (kind) {
    TileKind.Folder -> "dir"
    TileKind.Image -> "image"
    TileKind.Video -> "video"
    TileKind.Audio -> "audio"
    TileKind.Archive -> "archive"
    TileKind.Document -> "document"
    TileKind.Apps -> "apk"
    TileKind.Other, TileKind.Protected, TileKind.Estimated -> "other"
}
