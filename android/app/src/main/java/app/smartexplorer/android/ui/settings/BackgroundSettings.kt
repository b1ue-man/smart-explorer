// material3 1.4.0: several components may still be experimental (docs/refs/compose-material3.md §0).
@file:OptIn(ExperimentalMaterial3Api::class)

package app.smartexplorer.android.ui.settings

import android.Manifest
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.smartexplorer.android.api.BgStatus
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.api.SyncApi.displayText
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.service.BackgroundController
import app.smartexplorer.android.service.BackgroundText
import app.smartexplorer.android.system.Permissions
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.sync.BackgroundStatusBlock
import app.smartexplorer.android.ui.sync.CatchUpBlock
import app.smartexplorer.android.ui.sync.HintText
import app.smartexplorer.android.ui.sync.RepeatWhileStarted
import app.smartexplorer.android.ui.sync.SectionTitle
import app.smartexplorer.android.ui.sync.SwitchRow
import app.smartexplorer.android.ui.sync.WorkerLogDialog
import java.time.Duration
import java.time.ZonedDateTime
import kotlinx.coroutines.launch

private val INTERVALS = listOf(15, 30, 60, 180, 360)
private val MODES = listOf(
    BackgroundController.MODE_OFF,
    BackgroundController.MODE_PERIODIC,
    BackgroundController.MODE_PERSISTENT,
)
private const val STATUS_INTERVAL_MS = 5_000L

/**
 * Settings section "Hintergrund" (spec F17): mode, interval and conditions of the periodic run,
 * automatic pause, pause/resume, catch-up now, battery optimization, worker log and the Android
 * limits. Embedded by the settings page (K3) and the Sync page; it does not scroll by itself.
 */
@Composable
fun BackgroundSettingsSection(modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val mode by AppPrefs.bgMode.collectAsStateWithLifecycle()
    val nextRun by remember(context) { BackgroundController.nextRun(context) }.collectAsStateWithLifecycle(initialValue = null)
    var status by remember { mutableStateOf<BgStatus?>(null) }
    var statusError by remember { mutableStateOf<String?>(null) }
    var showLog by rememberSaveable { mutableStateOf(false) }

    val refresh: suspend () -> Unit = {
        try {
            status = SyncApi.status()
            statusError = null
        } catch (e: CoreException) {
            statusError = e.displayText()
        }
    }
    RepeatWhileStarted(STATUS_INTERVAL_MS, refresh)
    val control: (String, suspend () -> Unit) -> Unit = { failure, action ->
        scope.launch {
            try {
                action()
            } catch (e: CoreException) {
                Snackbars.show("$failure: ${e.displayText()}")
            }
            refresh()
        }
    }

    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        BackgroundStatusBlock(mode, status, nextRun, statusError)
        NotificationHint(mode)

        SectionTitle("Modus")
        SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
            MODES.forEachIndexed { index, value ->
                SegmentedButton(
                    selected = mode == value,
                    onClick = {
                        AppPrefs.setBgMode(value)
                        BackgroundController.apply(context)
                    },
                    shape = SegmentedButtonDefaults.itemShape(index = index, count = MODES.size),
                    // No check mark: three labels must fit a phone width.
                    icon = {},
                    label = { Text(BackgroundText.modeLabel(value), maxLines = 1) },
                )
            }
        }
        HintText(modeHint(mode))

        if (mode == BackgroundController.MODE_PERIODIC) PeriodicConditions { BackgroundController.apply(context) }

        SectionTitle("Automatische Pause")
        val current = status
        SwitchRow(
            "Bei Energiesparmodus",
            current?.autopauseBattery == true,
            { on -> control("Nicht geändert") { SyncApi.setAutopause(on, current?.autopauseMetered == true) } },
            enabled = current != null,
        )
        SwitchRow(
            "Bei getaktetem Netz",
            current?.autopauseMetered == true,
            { on -> control("Nicht geändert") { SyncApi.setAutopause(current?.autopauseBattery == true, on) } },
            enabled = current != null,
        )

        SectionTitle("Pause")
        PauseControls(current, control)

        SectionTitle("Nachholen")
        CatchUpBlock(onChanged = { scope.launch { refresh() } })

        SectionTitle("System")
        BatteryOptimizationRow(mode)
        OutlinedButton(onClick = { showLog = true }) { Text("Worker-Protokoll") }

        SectionTitle("Grenzen auf Android")
        HintText("Echtzeit-Jobs laufen im Dauerbetrieb oder bei geöffneter App; im Modus „Periodisch“ einmal je Lauf.")
        HintText("Zeitplan-Jobs sind im Modus „Periodisch“ nicht minutengenau; Android verschiebt Läufe.")
        HintText("Befehle vorher/nachher laufen nur bei Hintergrundläufen, mit /system/bin/sh in der App-Sandbox.")
        HintText("Der Auslöser „Bei Anschluss“ (Laufwerk/USB) wird auf Android nicht unterstützt.")
    }
    if (showLog) WorkerLogDialog(onDismiss = { showLog = false })
}

private fun modeHint(mode: String): String = when (mode) {
    BackgroundController.MODE_OFF -> "Keine geplanten Jobs; „Jetzt“ auf der Sync-Seite geht weiterhin."
    BackgroundController.MODE_PERSISTENT ->
        "Eine dauerhafte Benachrichtigung hält Jobs und Share wach, auch nach einem Neustart. Braucht mehr Akku."
    else -> "Android weckt die App regelmäßig unter den Bedingungen unten und holt fällige Jobs nach."
}

@Composable
private fun PeriodicConditions(onChanged: () -> Unit) {
    val interval by AppPrefs.bgIntervalMin.collectAsStateWithLifecycle()
    val wifiOnly by AppPrefs.bgWifiOnly.collectAsStateWithLifecycle()
    val chargingOnly by AppPrefs.bgChargingOnly.collectAsStateWithLifecycle()
    val batteryNotLow by AppPrefs.bgBatteryNotLow.collectAsStateWithLifecycle()
    SectionTitle("Periodischer Lauf")
    Text("Abstand", modifier = Modifier.fillMaxWidth())
    Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        INTERVALS.forEach { minutes ->
            FilterChip(
                selected = interval == minutes,
                onClick = {
                    AppPrefs.setBgIntervalMin(minutes)
                    onChanged()
                },
                label = { Text(if (minutes < 60) "$minutes min" else "${minutes / 60} h") },
            )
        }
    }
    val update: ((Boolean) -> Unit) -> (Boolean) -> Unit = { setter ->
        { value ->
            setter(value)
            onChanged()
        }
    }
    SwitchRow("Nur im WLAN (ungetaktet)", wifiOnly, update(AppPrefs::setBgWifiOnly))
    SwitchRow("Nur beim Laden", chargingOnly, update(AppPrefs::setBgChargingOnly))
    SwitchRow("Nicht bei niedrigem Akku", batteryNotLow, update(AppPrefs::setBgBatteryNotLow))
}

@Composable
private fun PauseControls(status: BgStatus?, control: (String, suspend () -> Unit) -> Unit) {
    if (status?.paused == true) {
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(BackgroundText.paused(status), modifier = Modifier.weight(1f))
            OutlinedButton(onClick = { control("Nicht fortgesetzt") { SyncApi.resume() } }) { Text("Fortsetzen") }
        }
        return
    }
    Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        val enabled = status != null
        OutlinedButton(onClick = { control("Nicht pausiert") { SyncApi.pause(3_600) } }, enabled = enabled) { Text("1 h") }
        OutlinedButton(onClick = { control("Nicht pausiert") { SyncApi.pause(secondsUntilTomorrow()) } }, enabled = enabled) {
            Text("Bis morgen")
        }
        OutlinedButton(onClick = { control("Nicht pausiert") { SyncApi.pause(-1) } }, enabled = enabled) { Text("Unbegrenzt") }
    }
}

/** Seconds until the start of the next local day. */
private fun secondsUntilTomorrow(): Long {
    val now = ZonedDateTime.now()
    val tomorrow = now.toLocalDate().plusDays(1).atStartOfDay(now.zone)
    return Duration.between(now, tomorrow).seconds.coerceAtLeast(60)
}

/** "Fehler: Benachrichtigungen verboten → Hinweis mit [Erlauben]" (spec F17). */
@Composable
private fun NotificationHint(mode: String) {
    val context = LocalContext.current
    var allowed by remember { mutableStateOf(Permissions.canPostNotifications(context)) }
    var asked by rememberSaveable { mutableStateOf(false) }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { allowed = Permissions.canPostNotifications(context) }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        allowed = Permissions.canPostNotifications(context)
    }
    if (allowed || mode == BackgroundController.MODE_OFF) return
    ErrorCard(
        message = "Ohne Benachrichtigungen sind Hintergrundläufe, Fortschritt und Share-Anfragen nicht sichtbar.",
        title = "Benachrichtigungen aus",
        actionLabel = "Erlauben",
        onAction = {
            if (Permissions.notificationsNeedRuntimePermission() && !asked) {
                asked = true
                launcher.launch(Manifest.permission.POST_NOTIFICATIONS)
            } else if (!Permissions.openNotificationSettings(context)) {
                Snackbars.show("Einstellung nicht verfügbar – bitte in den App-Einstellungen erlauben.")
            }
        },
    )
}

@Composable
private fun BatteryOptimizationRow(mode: String) {
    val context = LocalContext.current
    var exempt by remember { mutableStateOf(Permissions.isIgnoringBatteryOptimizations(context)) }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { exempt = Permissions.isIgnoringBatteryOptimizations(context) }
    if (exempt) {
        HintText("Akku-Optimierung ist für Smart Explorer ausgeschaltet.")
        return
    }
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        OutlinedButton(onClick = {
            if (!Permissions.requestIgnoreBatteryOptimizations(context)) {
                Snackbars.show("Einstellung nicht verfügbar – bitte in den Akku-Einstellungen ändern.")
            }
        }) { Text("Akku-Optimierung ausschalten") }
        HintText(
            if (mode == BackgroundController.MODE_PERSISTENT) {
                "Empfohlen im Dauerbetrieb: Android darf den Dienst dann auch aus dem Hintergrund neu starten."
            } else {
                "Erlaubt Android, Hintergrundläufe seltener zu verschieben."
            },
        )
    }
}
