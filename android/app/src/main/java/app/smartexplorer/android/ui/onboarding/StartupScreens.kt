package app.smartexplorer.android.ui.onboarding

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.os.Build
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.BuildConfig
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.Snackbars
import kotlinx.coroutines.delay

/** Shown while the core starts; the indicator appears only if the start takes noticeably long. */
@Composable
fun StartupScreen() {
    var showIndicator by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        delay(300)
        showIndicator = true
    }
    Box(Modifier.fillMaxSize().safeDrawingPadding(), contentAlignment = Alignment.Center) {
        if (showIndicator) {
            Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(16.dp)) {
                CircularProgressIndicator()
                Text("Kern wird gestartet …", style = MaterialTheme.typography.bodyMedium)
            }
        }
    }
}

/** The core could not start (spec F1): message, [Protokoll teilen], [Erneut]. */
@Composable
fun StartupErrorScreen(message: String, onRetry: () -> Unit) {
    val context = LocalContext.current
    Box(Modifier.fillMaxSize().safeDrawingPadding().padding(24.dp), contentAlignment = Alignment.Center) {
        Column(Modifier.widthIn(max = 560.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            ErrorCard(
                title = "Kern konnte nicht starten",
                message = "$message\n\nErneut versuchen; bleibt der Fehler, das Protokoll teilen und melden.",
            )
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp), modifier = Modifier.align(Alignment.End)) {
                OutlinedButton(onClick = { shareStartupLog(context, message) }) { Text("Protokoll teilen") }
                Button(onClick = onRetry) { Text("Erneut") }
            }
        }
    }
}

private fun shareStartupLog(context: Context, message: String) {
    val text = buildString {
        appendLine("Smart Explorer ${BuildConfig.VERSION_NAME} (${BuildConfig.VERSION_CODE})")
        appendLine("Android ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT}), ${Build.MANUFACTURER} ${Build.MODEL}")
        appendLine("ABI: ${Build.SUPPORTED_ABIS.joinToString()}")
        appendLine()
        appendLine("Kernstart fehlgeschlagen:")
        append(message)
    }
    val send = Intent(Intent.ACTION_SEND)
        .setType("text/plain")
        .putExtra(Intent.EXTRA_SUBJECT, "Smart Explorer – Startfehler")
        .putExtra(Intent.EXTRA_TEXT, text)
    try {
        context.startActivity(Intent.createChooser(send, "Protokoll teilen"))
    } catch (e: ActivityNotFoundException) {
        Snackbars.show("Keine App zum Teilen gefunden.")
    }
}
