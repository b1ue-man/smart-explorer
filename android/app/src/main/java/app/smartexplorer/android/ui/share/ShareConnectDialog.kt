package app.smartexplorer.android.ui.share

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
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
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.DiscoveryAdvert
import app.smartexplorer.android.api.SHARE_DIRECT
import app.smartexplorer.android.api.ShareApi
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.Snackbars
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

private const val DISCOVER_INTERVAL_MS = 10_000L
private const val CONNECT_TIMEOUT_MS = 60_000L

/**
 * "Gerät verbinden" (spec F18): discoverable devices and rooms (name, expiry) → tap → PIN →
 * "Verbinde …" (1–10 s, [Abbrechen]) → result. The list refreshes itself while the dialog is open.
 */
@Composable
internal fun ConnectDeviceDialog(vm: ShareViewModel, onDismiss: () -> Unit) {
    val scope = rememberCoroutineScope()
    var chosen by remember { mutableStateOf<DiscoveryAdvert?>(null) }
    var pin by remember { mutableStateOf("") }
    var connecting by remember { mutableStateOf(false) }
    // Exchange visible before [Verbinden]; only a newer one belongs to this attempt.
    var previousExchange by remember { mutableStateOf<String?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    val discovery = vm.status?.discovery
    val exchange = discovery?.exchange?.takeIf { connecting && it.exchangeId != previousExchange }

    LaunchedEffect(Unit) {
        while (true) {
            try {
                ShareApi.discover()
            } catch (e: CoreException) {
                failure = "Suche nicht gestartet: ${e.message ?: e.kind}"
            }
            vm.reload()
            delay(DISCOVER_INTERVAL_MS)
        }
    }
    LaunchedEffect(exchange?.exchangeId, exchange?.state) {
        val current = exchange ?: return@LaunchedEffect
        when (current.state) {
            "done" -> {
                Snackbars.show(chosen?.alias?.takeIf { it.isNotBlank() }?.let { "Verbunden mit $it" } ?: "Verbunden")
                onDismiss()
            }
            "failed" -> {
                connecting = false
                failure = current.message?.takeIf { it.isNotBlank() } ?: "Verbindung fehlgeschlagen – PIN prüfen."
            }
            "canceled" -> {
                connecting = false
                failure = "Abgebrochen"
            }
        }
    }
    LaunchedEffect(connecting) {
        if (!connecting) return@LaunchedEffect
        delay(CONNECT_TIMEOUT_MS)
        connecting = false
        failure = "Keine Antwort – bitte erneut versuchen."
    }

    val target = chosen
    AlertDialog(
        // While connecting, only [Abbrechen] ends the dialog.
        onDismissRequest = { if (!connecting) onDismiss() },
        title = { Text(if (target == null) "Gerät verbinden" else target.alias.ifBlank { "Gerät verbinden" }) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                when {
                    connecting -> {
                        Text("Verbinde …", style = MaterialTheme.typography.bodyMedium)
                        LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                    }
                    target != null -> OutlinedTextField(
                        value = pin,
                        onValueChange = { pin = it },
                        label = { Text("PIN des anderen Geräts") },
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                        modifier = Modifier.fillMaxWidth(),
                    )
                    else -> AdvertList(
                        discovery?.advertisements.orEmpty(),
                        onChoose = {
                            chosen = it
                            pin = ""
                            failure = null
                        },
                    )
                }
                failure?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium) }
            }
        },
        confirmButton = {
            when {
                connecting -> TextButton(onClick = {
                    val running = discovery?.exchange?.takeIf { it.exchangeId != previousExchange && it.state == "running" }
                    connecting = false
                    if (running != null) vm.act("Nicht abgebrochen") { ShareApi.cancelConnect(running.exchangeId) }
                }) { Text("Abbrechen") }
                target != null -> TextButton(
                    onClick = {
                        previousExchange = discovery?.exchange?.exchangeId
                        failure = null
                        connecting = true
                        scope.launch {
                            try {
                                ShareApi.connect(target.discoveryId, pin)
                            } catch (e: CoreException) {
                                connecting = false
                                failure = e.message ?: e.kind
                            }
                            vm.reload()
                        }
                    },
                    enabled = pin.isNotEmpty(),
                ) { Text("Verbinden") }
                else -> TextButton(onClick = onDismiss) { Text("Schließen") }
            }
        },
        dismissButton = {
            if (target != null && !connecting) {
                TextButton(onClick = {
                    chosen = null
                    failure = null
                }) { Text("Zurück") }
            }
        },
    )
}

@Composable
private fun AdvertList(adverts: List<DiscoveryAdvert>, onChoose: (DiscoveryAdvert) -> Unit) {
    if (adverts.isEmpty()) {
        Text(
            "Suche nach Geräten und Räumen … Auf dem anderen Gerät „Suchbar machen“ wählen.",
            style = MaterialTheme.typography.bodyMedium,
        )
        LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
        return
    }
    LazyColumn(Modifier.heightIn(max = 360.dp)) {
        items(adverts, key = { it.discoveryId }) { advert ->
            val kind = if (advert.kind == SHARE_DIRECT) "Gerät" else "Raum"
            val detail = if (advert.compatible) {
                "$kind · bis ${Format.time(advert.expiresMs)}"
            } else {
                "$kind · nicht kompatibel (andere Version)"
            }
            ListItem(
                headlineContent = { Text(advert.alias.ifBlank { kind }, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                supportingContent = { Text(detail) },
                leadingContent = {
                    SeIcon(if (advert.kind == SHARE_DIRECT) R.drawable.ic_device else R.drawable.ic_room, contentDescription = null)
                },
                modifier = Modifier.clickable(enabled = advert.compatible) { onChoose(advert) },
            )
        }
    }
}
