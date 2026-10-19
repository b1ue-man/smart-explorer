package app.smartexplorer.android.ui.screen

import android.content.Intent
import android.net.Uri
import android.provider.Settings
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import app.smartexplorer.android.api.ACCESSIBILITY_ON
import app.smartexplorer.android.api.ACCESSIBILITY_RESTRICTED
import app.smartexplorer.android.api.ScreenApi
import app.smartexplorer.android.api.ScreenHostSession
import app.smartexplorer.android.api.ScreenPrereqs
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.ScreenSessionChanged
import app.smartexplorer.android.screen.ScreenHostControl
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.more.HintLine
import app.smartexplorer.android.ui.more.SectionHeader
import kotlinx.coroutines.launch

/** The words of a session state of `screen.hostSessions`. */
internal fun screenSessionState(session: ScreenHostSession): String = when (session.state) {
    "consent" -> "wartet auf deine Freigabe"
    "streaming" -> if (session.still) "sieht Vorschaubilder" else if (session.projection) "sieht den Bildschirm" else "verbunden"
    else -> "beendet"
}

/**
 * Teilen → „Bildschirm dieses Telefons“ (plan 2026-10-17, SCR-M8): the running sessions with
 * „Beenden“, the state of the se accessibility service with „Aktivieren“ (and the hint on
 * „Eingeschränkte Einstellungen“ for a sideloaded app), and usage access for the app list. Who may
 * watch or control is set per device in „Zugriff…“; every session with screen content also needs
 * „Bildschirm teilen“ of Android.
 */
@Composable
fun ScreenHostSection() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var prereqs by remember { mutableStateOf<ScreenPrereqs?>(null) }
    var sessions by remember { mutableStateOf<List<ScreenHostSession>>(emptyList()) }
    suspend fun reload() {
        try {
            ScreenHostControl.start(context)
            ScreenHostControl.reportPrereqs()
            prereqs = ScreenApi.prereqs()
            sessions = ScreenApi.sessions()
        } catch (e: CoreException) {
            // The core starts later; the next `share` event or resume loads again.
        }
    }
    LaunchedEffect(Unit) {
        reload()
        Core.events.collect { if (it is ScreenSessionChanged) reload() }
    }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { scope.launch { reload() } }

    Column {
        SectionHeader("Bildschirm dieses Telefons")
        HintLine("Andere Geräte sehen oder steuern diesen Bildschirm nur mit deiner Erlaubnis; Android fragt jedes Mal „Bildschirm teilen“.")
        if (sessions.isEmpty()) HintLine("Gerade sieht niemand diesen Bildschirm.")
        sessions.forEach { session ->
            ListItem(
                headlineContent = { Text(session.viewerName.ifBlank { "Ein Gerät" }) },
                supportingContent = { Text(screenSessionState(session)) },
                trailingContent = {
                    TextButton(onClick = {
                        scope.launch {
                            try {
                                ScreenApi.stop(session.session)
                            } catch (e: CoreException) {
                                Snackbars.show("Nicht beendet: ${e.message ?: e.kind}")
                            }
                            reload()
                        }
                    }) { Text("Beenden") }
                },
            )
        }
        if (sessions.size > 1) {
            TextButton(onClick = { scope.launch { ScreenHostControl.stopAll(context); reload() } }, modifier = Modifier.padding(start = 4.dp)) {
                Text("Alle beenden")
            }
        }
        val accessibility = prereqs?.accessibility
        Text(
            "Bedienungshilfe: " + when (accessibility) {
                ACCESSIBILITY_ON -> "an (Steuern, Text, Vorschaubilder)"
                ACCESSIBILITY_RESTRICTED -> "aus – zuerst „Eingeschränkte Einstellungen zulassen“"
                else -> "aus (nur Ansehen nach Freigabe)"
            },
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
        )
        if (accessibility == ACCESSIBILITY_RESTRICTED) {
            HintLine("App-Info öffnen → ⋮ → „Eingeschränkte Einstellungen zulassen“, danach hier „Aktivieren“.")
            TextButton(onClick = {
                context.startActivity(
                    Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${context.packageName}"))
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
            }, modifier = Modifier.padding(start = 4.dp)) { Text("App-Info öffnen") }
        }
        if (accessibility != ACCESSIBILITY_ON) {
            TextButton(onClick = {
                context.startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }, modifier = Modifier.padding(start = 4.dp)) { Text("Aktivieren") }
        }
        Text(
            "Nutzungsdatenzugriff (App-Liste ohne Bedienungshilfe): " + if (prereqs?.usageAccess == true) "erlaubt" else "nicht erlaubt",
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
        )
        if (prereqs?.usageAccess != true) {
            TextButton(onClick = {
                context.startActivity(Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }, modifier = Modifier.padding(start = 4.dp)) { Text("Nutzungsdatenzugriff öffnen") }
        }
    }
}
