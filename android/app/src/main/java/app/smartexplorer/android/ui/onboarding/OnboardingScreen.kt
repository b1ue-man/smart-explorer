package app.smartexplorer.android.ui.onboarding

import android.Manifest
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.DrawableRes
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import app.smartexplorer.android.R
import app.smartexplorer.android.system.Permissions
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.common.rememberAllFilesAccess

/**
 * First-start page "Einrichtung" (spec F1): all-files access (system settings page) and
 * notifications (runtime dialog on Android 13+), then [onDone]. Both are optional; without file
 * access the file view shows only app folders plus a hint banner.
 */
@Composable
fun OnboardingScreen(onDone: () -> Unit) {
    val context = LocalContext.current
    val filesGranted = rememberAllFilesAccess()
    var notificationsGranted by remember { mutableStateOf(Permissions.canPostNotifications(context)) }
    var notificationsAsked by rememberSaveable { mutableStateOf(false) }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) {
        notificationsGranted = Permissions.canPostNotifications(context)
    }
    val notificationLauncher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        notificationsGranted = Permissions.canPostNotifications(context)
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .safeDrawingPadding()
            .verticalScroll(rememberScrollState())
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Column(Modifier.widthIn(max = 560.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text("Einrichtung", style = MaterialTheme.typography.headlineMedium)
            Text(
                "Smart Explorer arbeitet direkt mit den Dateien auf diesem Gerät. Zwei Freigaben " +
                    "machen das möglich; beide lassen sich später in den Einstellungen ändern.",
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            PermissionCard(
                icon = R.drawable.ic_folder,
                title = "Zugriff auf alle Dateien",
                text = "Nötig, um internen Speicher, SD-Karte und USB-Speicher zu durchsuchen, " +
                    "zu kopieren und zu synchronisieren.",
                granted = filesGranted,
                onAllow = {
                    if (!Permissions.openAllFilesAccessSettings(context)) {
                        Snackbars.show("Einstellung nicht verfügbar – bitte in den App-Einstellungen erlauben.")
                    }
                },
            )
            PermissionCard(
                icon = R.drawable.ic_info,
                title = "Benachrichtigungen",
                text = "Zeigen Fortschritt von Übertragungen und Sync-Läufen im Hintergrund sowie " +
                    "Anfragen anderer Geräte.",
                granted = notificationsGranted,
                onAllow = {
                    if (Permissions.notificationsNeedRuntimePermission() && !notificationsAsked) {
                        notificationsAsked = true
                        notificationLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
                    } else if (!Permissions.openNotificationSettings(context)) {
                        Snackbars.show("Einstellung nicht verfügbar – bitte in den App-Einstellungen erlauben.")
                    }
                },
            )
            Spacer(Modifier.height(8.dp))
            Button(onClick = onDone, modifier = Modifier.fillMaxWidth()) { Text("Weiter") }
        }
    }
}

@Composable
private fun PermissionCard(
    @DrawableRes icon: Int,
    title: String,
    text: String,
    granted: Boolean,
    onAllow: () -> Unit,
) {
    Card(Modifier.fillMaxWidth()) {
        Row(Modifier.padding(16.dp), horizontalArrangement = Arrangement.spacedBy(16.dp)) {
            SeIcon(icon, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(title, style = MaterialTheme.typography.titleMedium)
                Text(text, style = MaterialTheme.typography.bodyMedium)
                if (granted) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        SeIcon(R.drawable.ic_check, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
                        Text("Erlaubt", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary)
                    }
                } else {
                    FilledTonalButton(onClick = onAllow) { Text("Erlauben") }
                }
            }
        }
    }
}
