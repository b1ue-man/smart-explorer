package app.smartexplorer.android.launcher.ui

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.content.pm.LauncherApps
import android.os.Build
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyGridItemSpanScope
import androidx.compose.foundation.lazy.grid.LazyGridScope
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.KeyboardArrowUp
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.launcher.apps.AppEntry
import app.smartexplorer.android.launcher.apps.PrivateSpace
import app.smartexplorer.android.launcher.apps.ProfileInfo

/** The activity behind a composition's context, or `null`. */
private tailrec fun Context.activity(): Activity? = when (this) {
    is Activity -> this
    is ContextWrapper -> baseContext.activity()
    else -> null
}

private val FULL_ROW: LazyGridItemSpanScope.() -> GridItemSpan = { GridItemSpan(maxLineSpan) }

/**
 * The section „Privater Bereich“ below the grid of the tab „Persönlich“ (U38-14, P18): a header with a lock that folds
 * the section, then — locked — the line „Privater Bereich ist gesperrt.“ with „Entsperren“, or — unlocked — „Sperren“
 * and the apps of the profile. From API 36 the section also offers „Einstellungen des privaten Bereichs“. The system
 * itself asks for the device lock when unlocking; a refusal of `requestQuietModeEnabled` is told by the view model.
 * Call it only for a profile the person lets show and only while the section is wanted; a locked profile has no apps.
 */
fun LazyGridScope.privateSpaceSection(
    profile: ProfileInfo,
    apps: List<AppEntry>,
    expanded: Boolean,
    onToggle: () -> Unit,
    context: ItemContext,
    host: DragHost,
) {
    item(key = "private-header", span = FULL_ROW) { Header(expanded, onToggle) }
    if (!expanded) return
    if (profile.isQuiet) {
        item(key = "private-locked", span = FULL_ROW) {
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(16.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("Privater Bereich ist gesperrt.", Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
                Button(onClick = { context.vm.setQuiet(profile, false) }) { Text("Entsperren") }
            }
        }
    } else {
        item(key = "private-lock", span = FULL_ROW) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp), horizontalArrangement = Arrangement.End) {
                OutlinedButton(onClick = { context.vm.setQuiet(profile, true) }) { Text("Sperren") }
            }
        }
        items(apps, key = { "private-" + it.key }) { entry -> DrawerCell(entry, context, host) }
    }
    if (PrivateSpace.settingsEntryAvailable(Build.VERSION.SDK_INT)) {
        item(key = "private-settings", span = FULL_ROW) { SettingsRow(context) }
    }
}

@Composable
private fun Header(expanded: Boolean, onToggle: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Button, onClick = onToggle)
            .semantics { stateDescription = if (expanded) "aufgeklappt" else "zugeklappt" }
            .padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Default.Lock, contentDescription = null)
        Text("Privater Bereich", Modifier.weight(1f), style = MaterialTheme.typography.titleSmall)
        Icon(if (expanded) Icons.Default.KeyboardArrowUp else Icons.Default.KeyboardArrowDown, contentDescription = null)
    }
}

/** „Einstellungen des privaten Bereichs“ (API 36): the `IntentSender` of `getPrivateSpaceSettingsIntent` (assumption A49). */
@Composable
private fun SettingsRow(context: ItemContext) {
    val activity = LocalContext.current.activity()
    TextButton(
        onClick = {
            val launcherApps = activity?.getSystemService(LauncherApps::class.java)
            val sender = try {
                launcherApps?.getPrivateSpaceSettingsIntent()
            } catch (_: RuntimeException) {
                null
            }
            if (sender == null || activity == null) {
                context.vm.showToast("Die Einstellungen des privaten Bereichs sind nicht verfügbar.")
            } else {
                try {
                    activity.startIntentSender(sender, null, 0, 0, 0)
                } catch (_: Exception) {
                    context.vm.showToast("Die Einstellungen des privaten Bereichs sind nicht verfügbar.")
                }
            }
        },
        modifier = Modifier.padding(horizontal = 8.dp),
    ) { Text("Einstellungen des privaten Bereichs") }
}
