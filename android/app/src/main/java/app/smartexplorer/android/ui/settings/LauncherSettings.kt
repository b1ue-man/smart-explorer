package app.smartexplorer.android.ui.settings

import android.content.ActivityNotFoundException
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.app.NotificationManagerCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import app.smartexplorer.android.launcher.EnableResult
import app.smartexplorer.android.launcher.HomeNotice
import app.smartexplorer.android.launcher.HomeResolution
import app.smartexplorer.android.launcher.HomeRules
import app.smartexplorer.android.launcher.LauncherParts
import app.smartexplorer.android.launcher.LauncherState
import app.smartexplorer.android.launcher.LauncherSwitch
import app.smartexplorer.android.launcher.apps.PrivateSpace
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.more.HintLine
import app.smartexplorer.android.ui.more.SectionHeader
import app.smartexplorer.android.ui.more.ToggleSetting

/** What the card shows; read anew on every `onResume`, never stored (U35-5, U35-9). */
private data class CardData(
    val enabled: Boolean,
    val roleAvailable: Boolean,
    val roleHeld: Boolean,
    val home: HomeResolution,
    val notice: HomeNotice,
    val previousName: String?,
    val widgetOn: Boolean,
    val recovered: Boolean,
    /** `ACCESS_HIDDEN_PROFILES` is granted (it can be withdrawn by hand only, U38-15). */
    val hiddenProfilesGranted: Boolean,
    /** The person allowed the notification listener of the dots (U38-12). */
    val badgesOn: Boolean,
)

private fun readCard(context: Context): CardData {
    val state = LauncherSwitch.reconcile(context)
    val home = LauncherSwitch.resolveHome(context)
    val notice = if (state.enabled) HomeNotice.Empty else HomeRules.notice(state.previousHome, state.noticeDismissed, home)
    return CardData(
        enabled = state.enabled,
        roleAvailable = LauncherSwitch.isRoleAvailable(context),
        roleHeld = LauncherSwitch.isRoleHeld(context),
        home = home,
        notice = notice,
        previousName = state.previousHome?.let { LauncherSwitch.previousHomeLabel(context, it) },
        widgetOn = state.parts.widget,
        recovered = LauncherState.recoveredFile(context).exists(),
        hiddenProfilesGranted = context.checkSelfPermission(PrivateSpace.PERMISSION) == PackageManager.PERMISSION_GRANTED,
        badgesOn = NotificationManagerCompat.getEnabledListenerPackages(context).contains(context.packageName),
    )
}

/** The system page for the notification access of the dots; the list of all listeners when the detail page is missing. */
private fun openBadgeAccess(context: Context) {
    val listener = ComponentName(context, BADGE_SERVICE).flattenToString()
    try {
        context.startActivity(
            Intent(Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS)
                .putExtra(Settings.EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME, listener)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        )
    } catch (_: ActivityNotFoundException) {
        try {
            context.startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        } catch (_: ActivityNotFoundException) {
            Snackbars.show("Einstellung nicht verfügbar – bitte in den Systemeinstellungen den Benachrichtigungszugriff erlauben.")
        }
    }
}

/** Class name of the listener; a string so the main process loads no launcher class (U35-2). */
private const val BADGE_SERVICE = "app.smartexplorer.android.launcher.badges.BadgeListenerService"

private fun openHomeSettings(context: Context) {
    try {
        context.startActivity(Intent(Settings.ACTION_HOME_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    } catch (_: ActivityNotFoundException) {
        Snackbars.show("Einstellung nicht verfügbar – bitte in den Systemeinstellungen den Startbildschirm wählen.")
    }
}

/**
 * Card „Startbildschirm“ (DSK-M9, U35-13): switch the se launcher on, off or away. Off is not
 * removal: „Deaktivieren“ keeps the arrangement, „Entfernen“ deletes it (after a question). All
 * state is read in [LifecycleEventEffect] on `ON_RESUME`; there is no polling loop.
 */
@Composable
fun LauncherSettings(modifier: Modifier = Modifier) {
    val context = LocalContext.current
    var card by remember { mutableStateOf(readCard(context)) }
    var confirmRemove by remember { mutableStateOf(false) }
    fun refresh() {
        card = readCard(context)
    }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { refresh() }
    val roleResult = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { refresh() }

    fun requestRole() {
        val intent = LauncherSwitch.requestHomeRole(context)
        if (intent != null) roleResult.launch(intent) else refresh()
    }

    Column(modifier) {
        SectionHeader("Startbildschirm")
        HintLine(
            "Der se Launcher ersetzt deinen Startbildschirm durch Apps, Widgets und deine Geräte, Räume und Tresore. " +
                "Du kannst ihn jederzeit deaktivieren oder entfernen.",
        )
        Column(
            Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            if (!card.enabled) {
                Button(
                    onClick = {
                        when (val result = LauncherSwitch.enable(context)) {
                            is EnableResult.Failed -> {
                                Snackbars.show(result.message)
                                refresh()
                            }
                            EnableResult.Done -> {
                                refresh()
                                requestRole()
                            }
                        }
                    },
                    modifier = Modifier.fillMaxWidth(),
                ) { Text("Aktivieren") }
                DisabledStatement(card.notice, card.previousName, context) { refresh() }
            } else {
                EnabledRows(card, context, ::requestRole)
                OutlinedButton(
                    onClick = {
                        val result = LauncherSwitch.disable(context)
                        result.error?.let { Snackbars.show("Deaktivieren fehlgeschlagen: $it") }
                        refresh()
                    },
                    modifier = Modifier.fillMaxWidth(),
                ) { Text("Deaktivieren") }
                OutlinedButton(onClick = { confirmRemove = true }, modifier = Modifier.fillMaxWidth()) { Text("Entfernen") }
            }
        }
        PrivateSpaceLine(card)
        if (card.enabled) BadgeLine(card, context)
        if (LauncherParts.widget.isNotEmpty()) {
            ToggleSetting("Geräte-Widget für fremde Launcher", card.widgetOn, { on ->
                try {
                    LauncherSwitch.setWidgetPart(context, on)
                } catch (e: Exception) {
                    Snackbars.show("Nicht gespeichert: ${e.message ?: e.javaClass.simpleName}")
                }
                refresh()
            })
        }
        if (card.recovered) {
            HintLine("Die gespeicherte Anordnung war unlesbar und wurde zur Seite gelegt.")
            TextButton(
                onClick = {
                    LauncherState.recoveredFile(context).delete()
                    refresh()
                },
                modifier = Modifier.padding(horizontal = 8.dp),
            ) { Text("Verstanden") }
        }
    }

    if (confirmRemove) {
        AlertDialog(
            onDismissRequest = { confirmRemove = false },
            title = { Text("Anordnung löschen?") },
            text = { Text("Die Anordnung des se Launchers (Seiten, Ordner, Widgets) wird gelöscht und der Launcher deaktiviert.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmRemove = false
                    val result = LauncherSwitch.remove(context)
                    (result.deleteError ?: result.error?.let { "Entfernen fehlgeschlagen: $it" })?.let { Snackbars.show(it) }
                    refresh()
                }) { Text("Entfernen") }
            },
            dismissButton = { TextButton(onClick = { confirmRemove = false }) { Text("Abbrechen") } },
        )
    }
}

/** Lines and buttons of the state „an“: default or not (U35-13). */
@Composable
private fun EnabledRows(card: CardData, context: Context, requestRole: () -> Unit) {
    if (card.roleHeld) {
        Text("Standard-Startbildschirm: Ja", style = MaterialTheme.typography.bodyLarge)
        return
    }
    Text(
        "Standard-Startbildschirm: Nein. Die Home-Taste öffnet einen anderen Startbildschirm.",
        style = MaterialTheme.typography.bodyLarge,
    )
    if (card.roleAvailable) {
        Button(onClick = requestRole, modifier = Modifier.fillMaxWidth()) { Text("Als Standard festlegen") }
    } else {
        Text("Dieses Gerät bietet keine Rollenabfrage. Wähle den Startbildschirm in den Systemeinstellungen.", style = MaterialTheme.typography.bodyMedium)
    }
    OutlinedButton(onClick = { openHomeSettings(context) }, modifier = Modifier.fillMaxWidth()) { Text("Systemeinstellung öffnen") }
}

/** The statement below „Aktivieren“ after a deactivation (U35-9). */
@Composable
private fun DisabledStatement(notice: HomeNotice, previousName: String?, context: Context, refresh: () -> Unit) {
    when (notice) {
        is HomeNotice.Current -> Text("Startbildschirm: ${notice.label}", style = MaterialTheme.typography.bodyLarge)
        is HomeNotice.ChooseAgain -> {
            Text(
                "Wähle deinen Startbildschirm. Vorher: ${previousName ?: "unbekannt"}.",
                style = MaterialTheme.typography.bodyLarge,
            )
            Button(onClick = { openHomeSettings(context) }, modifier = Modifier.fillMaxWidth()) { Text("Startbildschirm wählen") }
            OutlinedButton(
                onClick = {
                    LauncherSwitch.dismissNotice(context)
                    refresh()
                },
                modifier = Modifier.fillMaxWidth(),
            ) { Text("Verstanden") }
        }
        HomeNotice.Empty -> Unit
    }
}

@Composable
private fun PrivateSpaceLine(card: CardData) {
    val text = PrivateSpace.statusLine(Build.VERSION.SDK_INT, card.roleHeld, card.hiddenProfilesGranted)
    Text(text, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp))
}

/** „Benachrichtigungspunkte: an“ or „… aus (Zugriff fehlt)“ with the button to the system page (U38-12, U38-15). */
@Composable
private fun BadgeLine(card: CardData, context: Context) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(
            if (card.badgesOn) "Benachrichtigungspunkte: an" else "Benachrichtigungspunkte: aus (Zugriff fehlt)",
            style = MaterialTheme.typography.bodyMedium,
        )
        if (!card.badgesOn) {
            OutlinedButton(onClick = { openBadgeAccess(context) }, modifier = Modifier.fillMaxWidth()) { Text("Zugriff erlauben") }
        }
    }
}
