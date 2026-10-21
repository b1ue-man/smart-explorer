package app.smartexplorer.android.ui.screen

import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material3.Checkbox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
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
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.ScreenWindows

/** What the viewer asks the host for besides the picture: the shown source and the two quality switches. */
data class ViewSettings(val source: String? = null, val sharpness: Boolean = false, val lossless: Boolean = false)

private val ACTION_LABELS = mapOf(
    "back" to R.string.screen_action_back,
    "home" to R.string.screen_action_home,
    "recents" to R.string.screen_action_recents,
    "notifications" to R.string.screen_action_notifications,
    "quick_settings" to R.string.screen_action_quick_settings,
    "lock_screen" to R.string.screen_action_lock_screen,
    "power_dialog" to R.string.screen_action_power_dialog,
)

/** HID usages of the keys without a keyboard event (`TOOLBAR_KEYS` of the core) and their labels. */
private val KEY_LABELS = mapOf(
    0xE3 to R.string.screen_key_super_left,
    0x65 to R.string.screen_key_menu,
    0x46 to R.string.screen_key_print_screen,
    0x48 to R.string.screen_key_pause,
    0x47 to R.string.screen_key_scroll_lock,
    0x39 to R.string.screen_key_caps_lock,
    0x53 to R.string.screen_key_num_lock,
    (0x8000 or 0xCD) to R.string.screen_key_play_pause,
    (0x8000 or 0xE9) to R.string.screen_key_volume_up,
    (0x8000 or 0xEA) to R.string.screen_key_volume_down,
    (0x8000 or 0xE2) to R.string.screen_key_mute,
)

/** The top bar of the full view: device, status and the buttons „Steuern“, „Fenster“, „Aktionen“, „Tastatur“, „⋮“ and „Schließen“. */
@Composable
internal fun ScreenTopBar(
    title: String,
    status: String,
    ui: ScreenSessionState,
    controlOn: Boolean,
    settings: ViewSettings,
    zoomMode: Boolean,
    onControl: (Boolean) -> Unit,
    onSource: (String?) -> Unit,
    onListWindows: () -> Unit,
    onAction: (String) -> Unit,
    onUsage: (Int) -> Unit,
    onKeyboard: () -> Unit,
    onZoomMode: () -> Unit,
    onSettings: (ViewSettings) -> Unit,
    onStatus: (() -> Unit)?,
    onClose: () -> Unit,
) {
    Surface(color = Color.Black.copy(alpha = 0.6f), contentColor = Color.White) {
        Column(Modifier.statusBarsPadding().padding(horizontal = 8.dp, vertical = 4.dp)) {
            Row(Modifier.horizontalScroll(rememberScrollState()), verticalAlignment = Alignment.CenterVertically) {
                Text(title, style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(end = 8.dp))
                if (ui.canControl) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(stringResource(R.string.screen_view_control))
                        Switch(checked = controlOn, onCheckedChange = onControl, enabled = ui.holder != "other", modifier = Modifier.padding(horizontal = 4.dp))
                    }
                }
                WindowsMenu(ui, settings.source, onSource, onListWindows)
                if (controlOn) ActionsMenu(ui, zoomMode, onAction, onUsage, onZoomMode)
                if (controlOn && (ui.input.text || ui.input.keyboard)) {
                    TextButton(onClick = onKeyboard) { Text(stringResource(R.string.screen_view_keyboard)) }
                }
                SettingsMenu(settings, onSettings)
                TextButton(onClick = onClose, modifier = Modifier.testTag("screen_view_close")) { Text(stringResource(R.string.screen_view_close)) }
            }
            if (status.isNotEmpty()) {
                val modifier = if (onStatus != null) Modifier.clickable(onClick = onStatus) else Modifier
                Text(status, style = MaterialTheme.typography.bodySmall, modifier = modifier)
            }
        }
    }
}

@Composable
private fun WindowsMenu(ui: ScreenSessionState, source: String?, onSource: (String?) -> Unit, onList: () -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        TextButton(onClick = { open = true; onList() }) { Text(stringResource(R.string.screen_view_windows)) }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            val pick = { id: String? -> open = false; onSource(id) }
            DropdownMenuItem(text = { Text(stringResource(R.string.screen_view_whole_screen)) }, onClick = { pick(null) })
            ui.monitors.forEachIndexed { index, monitor ->
                val label = if (monitor.width == 0) stringResource(R.string.screen_view_monitor_pick_on_host) else monitor.name
                DropdownMenuItem(
                    text = { Text(stringResource(R.string.screen_view_monitor, index + 1, label)) },
                    onClick = { pick("monitor:${monitor.id}") },
                )
            }
            if ("wayland_window_pick_on_host" in ui.limits) {
                Text(stringResource(R.string.screen_view_window_pick_on_host), Modifier.padding(horizontal = 16.dp, vertical = 8.dp))
            } else {
                windowEntries(ui.windows).forEach { (label, id) ->
                    DropdownMenuItem(text = { Text(label, maxLines = 1, overflow = TextOverflow.Ellipsis) }, onClick = { pick(id) })
                }
            }
            DropdownMenuItem(text = { Text(stringResource(R.string.screen_view_windows_refresh)) }, onClick = onList)
        }
    }
}

/** (label, source) of every window and app of a window list; a window is „{title} ({app})“. */
@Composable
private fun windowEntries(list: ScreenWindows?): List<Pair<String, String>> {
    val windows = list?.windows.orEmpty().map { stringResource(R.string.screen_view_window_entry, it.title, it.app) to "window:${it.id}" }
    val apps = list?.apps.orEmpty().map { it.name.ifBlank { it.appId } to "window:${it.appId}" }
    return windows + apps
}

@Composable
private fun ActionsMenu(
    ui: ScreenSessionState,
    zoomMode: Boolean,
    onAction: (String) -> Unit,
    onUsage: (Int) -> Unit,
    onZoomMode: () -> Unit,
) {
    val keys = ui.input.toolbarKeys.filter { it in KEY_LABELS }
    val actions = ui.input.actions.filter { it in ACTION_LABELS }
    val android = ui.platform == "android"
    if (keys.isEmpty() && actions.isEmpty() && !android) return
    var open by remember { mutableStateOf(false) }
    Box {
        TextButton(onClick = { open = true }) { Text(stringResource(R.string.screen_view_actions)) }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            actions.forEach { action ->
                DropdownMenuItem(text = { Text(stringResource(ACTION_LABELS.getValue(action))) }, onClick = { open = false; onAction(action) })
            }
            keys.forEach { usage ->
                DropdownMenuItem(text = { Text(stringResource(KEY_LABELS.getValue(usage))) }, onClick = { open = false; onUsage(usage) })
            }
            if (android) {
                // Two fingers cannot both zoom and touch: this switch turns the view into a zoom mode.
                DropdownMenuItem(
                    text = { Text(stringResource(R.string.screen_view_zoom_mode)) },
                    trailingIcon = { Checkbox(checked = zoomMode, onCheckedChange = null) },
                    onClick = { open = false; onZoomMode() },
                )
            }
        }
    }
}

@Composable
private fun SettingsMenu(settings: ViewSettings, onSettings: (ViewSettings) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        TextButton(onClick = { open = true }) { Text("⋮") }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            DropdownMenuItem(
                text = { Text(stringResource(R.string.screen_view_sharpness)) },
                trailingIcon = { Checkbox(checked = settings.sharpness, onCheckedChange = null) },
                onClick = { onSettings(settings.copy(sharpness = !settings.sharpness)) },
            )
            DropdownMenuItem(
                text = { Text(stringResource(R.string.screen_view_lossless)) },
                trailingIcon = { Checkbox(checked = settings.lossless, onCheckedChange = null) },
                onClick = { onSettings(settings.copy(lossless = !settings.lossless)) },
            )
        }
    }
}
