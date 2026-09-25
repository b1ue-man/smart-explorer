// material3 1.4.0: TopAppBar may still be experimental (docs/refs/compose-material3.md §0).
@file:OptIn(ExperimentalMaterial3Api::class)

package app.smartexplorer.android.ui.more

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.smartexplorer.android.R
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.system.Opener
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.Snackbars

// Building blocks shared by the pages of "Mehr", "Teilen", connections, analysis and settings.

/** Project pages linked from "Über", the Google Drive setup and error hints. */
internal object ProjectLinks {
    const val REPOSITORY = "https://github.com/b1ue-man/smart-explorer"
    const val README_ANDROID_LIMITS = "$REPOSITORY#android-umfang-und-grenzen"
    const val README_ANDROID_INSTALL = "$REPOSITORY#android-installieren-und-updates"
    const val CLOUD_SETUP = "$REPOSITORY/blob/main/docs/CLOUD_SETUP.md"
    const val DISCLAIMER = "$REPOSITORY/blob/main/DISCLAIMER.txt"
}

/** Browser links and plain-text sharing (android-apis.md §9.4, §6.4); failures end in a snackbar. */
internal object TextActions {
    fun openUrl(context: Context, url: String) {
        start(context, Intent(Intent.ACTION_VIEW, Uri.parse(url)), "Kein Browser gefunden.")
    }

    fun share(context: Context, subject: String, text: String) {
        val send = Intent(Intent.ACTION_SEND)
            .setType("text/plain")
            .putExtra(Intent.EXTRA_SUBJECT, subject)
            .putExtra(Intent.EXTRA_TEXT, text)
        start(context, Intent.createChooser(send, subject), "Keine App zum Teilen gefunden.")
    }

    fun copy(context: Context, label: String, text: String) {
        Snackbars.show(if (Opener.copyText(context, label, text)) "Kopiert" else "Zwischenablage nicht verfügbar")
    }

    private fun start(context: Context, intent: Intent, failure: String) {
        if (context !is Activity) intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        try {
            context.startActivity(intent)
        } catch (e: ActivityNotFoundException) {
            Snackbars.show(failure)
        }
    }
}

/** Page with a top bar and an optional back arrow; content gets the scaffold padding. */
@Composable
internal fun SubPageScaffold(
    title: String,
    onBack: (() -> Unit)?,
    actions: @Composable RowScope.() -> Unit = {},
    floatingActionButton: @Composable () -> Unit = {},
    content: @Composable (PaddingValues) -> Unit,
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                navigationIcon = {
                    if (onBack != null) {
                        IconButton(onClick = onBack) { SeIcon(R.drawable.ic_arrow_back, contentDescription = "Zurück") }
                    }
                },
                actions = actions,
            )
        },
        floatingActionButton = floatingActionButton,
        content = content,
    )
}

@Composable
internal fun SectionHeader(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.primary,
        modifier = modifier.padding(start = 16.dp, end = 16.dp, top = 20.dp, bottom = 4.dp),
    )
}

/** Explanatory line below a control (spec C: explanations only in hint lines). */
@Composable
internal fun HintLine(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = modifier.padding(horizontal = 16.dp, vertical = 2.dp),
    )
}

/** Whole-row switch (the row owns the click, compose-material3.md §15). */
@Composable
internal fun ToggleSetting(
    label: String,
    checked: Boolean,
    onChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .toggleable(value = checked, enabled = enabled, role = Role.Switch, onValueChange = onChange)
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(label, modifier = Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
        Switch(checked = checked, onCheckedChange = null, enabled = enabled)
    }
}

/** Latest state of task [taskId] from `Core.tasks`; `null` before its first event. */
@Composable
internal fun rememberTask(taskId: String?): TaskInfo? {
    val tasks by Core.tasks.collectAsStateWithLifecycle()
    return taskId?.let { id -> tasks.firstOrNull { it.id == id } }
}

/** Long selectable text (reports, logs) with [Kopieren] and [Teilen]. */
@Composable
internal fun TextReportDialog(title: String, text: String, onDismiss: () -> Unit) {
    val context = LocalContext.current
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            SelectionContainer {
                Text(
                    text.ifBlank { "–" },
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    modifier = Modifier.heightIn(max = 420.dp).verticalScroll(rememberScrollState()),
                )
            }
        },
        confirmButton = {
            Row {
                TextButton(onClick = { TextActions.copy(context, title, text) }) { Text("Kopieren") }
                TextButton(onClick = { TextActions.share(context, "Smart Explorer – $title", text) }) { Text("Teilen") }
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Schließen") } },
    )
}
