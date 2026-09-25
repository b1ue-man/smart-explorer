package app.smartexplorer.android.ui.settings

import android.os.Build
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.BuildConfig
import app.smartexplorer.android.R
import app.smartexplorer.android.api.CoreInfo
import app.smartexplorer.android.api.SysApi
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.more.ProjectLinks
import app.smartexplorer.android.ui.more.SectionHeader
import app.smartexplorer.android.ui.more.SubPageScaffold
import app.smartexplorer.android.ui.more.TextActions

/** Open points and Android limits (spec A "Nicht-Ziele"), in short. Details: README. */
private val LIMITS = listOf(
    "Google-Drive-Anmeldung aus dem Android-Browser ist ohne echtes Konto nicht geprüft; scheitert sie, " +
        "bleibt Drive am Telefon nicht nutzbar.",
    "Andere Geräte erreichen das Telefon über Share nur bei offener App, laufender Übertragung oder im Dauerbetrieb.",
    "Echtzeit-Jobs laufen im Dauerbetrieb oder bei offener App; Zeitplan-Jobs sind im Modus „Periodisch“ nicht minutengenau.",
    "Kein Einbinden als Laufwerk, keine Netzlaufwerke (SMB), keine Befehle anderer Geräte auf dem Telefon.",
    "Updates kommen über den Update-Feed und GitHub-Releases (keine Play-Store-Version, kein Rollback).",
)

/**
 * "Über" (spec F21, umsetzung K3): app and core version, links to the README sections "Android:
 * Umfang und Grenzen" and "Installieren und Updates", the Drive setup guide, license and notice,
 * and the open points.
 */
@Composable
fun AboutScreen(onBack: () -> Unit) {
    val context = LocalContext.current
    var info by remember { mutableStateOf<CoreInfo?>(null) }
    var infoError by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(Unit) {
        try {
            info = SysApi.info()
        } catch (e: CoreException) {
            infoError = e.message ?: e.kind
        }
    }
    val link: (String) -> Unit = { url -> TextActions.openUrl(context, url) }

    SubPageScaffold(title = "Über", onBack = onBack) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState())) {
            ListItem(
                headlineContent = { Text("Smart Explorer für Android", style = MaterialTheme.typography.titleMedium) },
                supportingContent = {
                    Column {
                        Text("App-Version ${BuildConfig.VERSION_NAME} (${BuildConfig.VERSION_CODE})")
                        Text(
                            info?.coreVersion?.let { "Kernversion $it" }
                                ?: infoError?.let { "Kernversion unbekannt: $it" }
                                ?: "Kernversion wird geladen …",
                        )
                        Text("Android ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT}) · ${Build.SUPPORTED_ABIS.firstOrNull().orEmpty()}")
                    }
                },
                leadingContent = { SeIcon(R.drawable.ic_folder, contentDescription = null) },
            )

            SectionHeader("Dokumentation")
            LinkItem("Umfang und Grenzen (README)", ProjectLinks.README_ANDROID_LIMITS, link)
            LinkItem("Installieren und Updates (README)", ProjectLinks.README_ANDROID_INSTALL, link)
            LinkItem("Google Drive einrichten", ProjectLinks.CLOUD_SETUP, link)
            LinkItem("Projektseite und Lizenz", ProjectLinks.REPOSITORY, link)
            LinkItem("Hinweis und Haftungsausschluss", ProjectLinks.DISCLAIMER, link)

            SectionHeader("Offene Punkte und Grenzen auf Android")
            LIMITS.forEach { text ->
                Text(
                    "• $text",
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                )
            }

            SectionHeader("Symbole")
            Text(
                "Material Symbols von Google, Apache License 2.0.",
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 4.dp, bottom = 24.dp),
            )
        }
    }
}

@Composable
private fun LinkItem(title: String, url: String, open: (String) -> Unit) {
    ListItem(
        headlineContent = { Text(title) },
        supportingContent = { Text(url.removePrefix("https://"), style = MaterialTheme.typography.bodySmall) },
        leadingContent = { SeIcon(R.drawable.ic_link, contentDescription = null) },
        modifier = Modifier.clickable { open(url) },
    )
}
