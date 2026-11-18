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
import app.smartexplorer.android.screen.ScreenIconRef
import app.smartexplorer.android.screen.ScreenIconTarget

/**
 * What the viewer asks the host for besides the picture: the shown source and the [choice] of codec, decoder and
 * highest data rate. The old switches „Verlustfrei“ and „Schärfe“ are the codec choices `lossless` and `sharp`
 * („Scharf (GPU)“, the same `SET_MODE` bit `sharpness`, as on the desktop viewer); both are stored with the codec.
 */
data class ViewSettings(
    val source: String? = null,
    val choice: ViewerCodecChoice = ViewerCodecChoice(),
    /** „Monitor“ and „Auflösung“ (P5/P9): live to a host that answers `setDisplay`, else with a new session. */
    val display: ViewerDisplayChoice = ViewerDisplayChoice(),
) {
    val lossless: Boolean get() = choice.lossless

    /** `SET_MODE` `sharpness`: only the codec „Scharf (GPU)“ sets it. */
    val sharpness: Boolean get() = choice.sharp
}

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
    /** Codec, decoder stage and the answer to „Höchste Datenrate“ of the running picture chain (one line, may be empty). */
    info: String = "",
    /** „Monitor“/„Auflösung“: chosen, what the host did and why ([ScreenDisplayReportText.line]). */
    displayLine: String = "",
    nativePixels: Boolean = false,
    onNativePixels: () -> Unit = {},
    immersive: Boolean = false,
    onFullscreen: () -> Unit = {},
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
                TextButton(onClick = onNativePixels) { Text(if (nativePixels) "Einpassen" else "1:1") }
                TextButton(onClick = onFullscreen) { Text(if (immersive) "Vollbild verlassen" else "Vollbild") }
                SettingsMenu(settings, onSettings, displayLine)
                TextButton(onClick = onClose, modifier = Modifier.testTag("screen_view_close")) { Text(stringResource(R.string.screen_view_close)) }
            }
            if (immersive) Text("Zurück oder Randgeste · Strg+Alt+Pause · Strg+Alt+Umschalt+F · F11", style = MaterialTheme.typography.bodySmall)
            ui.codecFallback?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
            if (info.isNotEmpty()) Text(info, style = MaterialTheme.typography.bodySmall, modifier = Modifier.testTag("screen_view_info"))
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
            val systemSelected = "android_system_selected_source" in ui.limits
            DropdownMenuItem(text = { Text(if (systemSelected) "Android-Systemauswahl" else stringResource(R.string.screen_view_whole_screen)) }, onClick = { pick(null) })
            ui.monitors.forEachIndexed { index, monitor ->
                val label = if (monitor.width == 0) stringResource(R.string.screen_view_monitor_pick_on_host) else monitor.name
                DropdownMenuItem(
                    text = { Text(stringResource(R.string.screen_view_monitor, index + 1, label)) },
                    onClick = { pick("monitor:${monitor.id}") },
                )
            }
            if ("wayland_window_pick_on_host" in ui.limits) {
                Text(stringResource(R.string.screen_view_window_pick_on_host), Modifier.padding(horizontal = 16.dp, vertical = 8.dp))
            }
            if (systemSelected) Text("Die Quelle wird auf dem Android-Gerät im Systemdialog ausgewählt.", Modifier.padding(horizontal = 16.dp, vertical = 8.dp))
            windowEntries(ui.windows).forEach { entry ->
                DropdownMenuItem(
                    text = { Text(entry.label, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    leadingIcon = {
                        ScreenAppIcon(
                            peer = ui.iconPeer,
                            target = entry.target,
                            reference = entry.icon.takeIf { ui.iconVersion == 1 },
                            fetch = { request -> ui.iconFetch?.invoke(request) },
                        )
                    },
                    enabled = !systemSelected && !ui.limits.contains("wayland_window_pick_on_host"),
                    onClick = { pick(entry.source) },
                )
            }
            DropdownMenuItem(text = { Text(stringResource(R.string.screen_view_windows_refresh)) }, onClick = onList)
        }
    }
}

private data class WindowIconEntry(val label: String, val source: String, val target: ScreenIconTarget, val icon: ScreenIconRef?)

/** Actual window/package target, label and reference remain one row. */
@Composable
private fun windowEntries(list: ScreenWindows?): List<WindowIconEntry> {
    val windows = list?.windows.orEmpty().map {
        WindowIconEntry(stringResource(R.string.screen_view_window_entry, it.title, it.app), "window:${it.id}", ScreenIconTarget.window(it.id), it.icon)
    }
    val apps = list?.apps.orEmpty().map { WindowIconEntry(it.name.ifBlank { it.appId }, "window:${it.appId}", ScreenIconTarget.app(it.appId), it.icon) }
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
private fun SettingsMenu(settings: ViewSettings, onSettings: (ViewSettings) -> Unit, displayLine: String) {
    var open by remember { mutableStateOf(false) }
    var rateDialog by remember { mutableStateOf(false) }
    var sizeDialog by remember { mutableStateOf(false) }
    if (sizeDialog) DisplaySizeDialog(settings.display, onDismiss = { sizeDialog = false }) { width, height ->
        sizeDialog = false
        onSettings(settings.copy(display = settings.display.copy(width = width, height = height)))
    }
    // The dialog lives beside the menu: closing the menu must not remove it.
    if (rateDialog) MaxRateDialog(settings.choice.maxRateBps, onDismiss = { rateDialog = false }) { bps ->
        rateDialog = false
        onSettings(settings.copy(choice = settings.choice.copy(maxRateBps = bps)))
    }
    Box {
        TextButton(onClick = { open = true }) { Text("⋮") }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            // „Codec“ (with „Scharf (GPU)“ and „Verlustfrei“ in the place of the old switches), „Dekodierer“ and
            // „Höchste Datenrate“ (BK7b, P9).
            CodecMenuItems(settings, onSettings, onRate = { open = false; rateDialog = true })
            // „Monitor“ and „Auflösung“ of the host's display (P5/P9).
            DisplayMenuItems(settings, onSettings, onCustom = { open = false; sizeDialog = true }, stateLine = displayLine)
        }
    }
}
