package app.smartexplorer.android.ui.settings

import android.os.Build
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.BuildConfig
import app.smartexplorer.android.R
import app.smartexplorer.android.api.CoreInfo
import app.smartexplorer.android.api.LicensesApi
import app.smartexplorer.android.api.RustLicenses
import app.smartexplorer.android.api.SysApi
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.more.ProjectLinks
import app.smartexplorer.android.ui.more.SectionHeader
import app.smartexplorer.android.ui.more.SubPageScaffold
import app.smartexplorer.android.ui.more.TextActions
import kotlinx.coroutines.launch

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
 * the license texts of the Rust crates (plan C4) and the open points.
 */
@Composable
fun AboutScreen(onBack: () -> Unit) {
    var showLicenses by rememberSaveable { mutableStateOf(false) }
    if (showLicenses) {
        RustLicensesScreen(onBack = { showLicenses = false })
        return
    }
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

            SectionHeader("Lizenzen von Drittanbietern")
            ListItem(
                headlineContent = { Text("Lizenzen der Rust-Bibliotheken") },
                supportingContent = {
                    Text("Lizenztexte aller Rust-Crates im App-Kern", style = MaterialTheme.typography.bodySmall)
                },
                modifier = Modifier.clickable { showLicenses = true },
            )

            SectionHeader("Symbole")
            Text(
                "Material Symbols von Google, Apache License 2.0.",
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 4.dp, bottom = 24.dp),
            )
        }
    }
}

/**
 * "Lizenzen der Rust-Bibliotheken" (plan C4): the license file embedded in the native library
 * (`app.licenses`), line by line, with a search that jumps between the matching lines. Without an
 * embedded file (development build) it shows the notice of the core.
 */
@Composable
private fun RustLicensesScreen(onBack: () -> Unit) {
    BackHandler(onBack = onBack)
    val scope = rememberCoroutineScope()
    var licenses by remember { mutableStateOf<RustLicenses?>(null) }
    var loadError by remember { mutableStateOf<String?>(null) }
    var reloadKey by remember { mutableIntStateOf(0) }
    LaunchedEffect(reloadKey) {
        try {
            licenses = LicensesApi.licenses()
            loadError = null
        } catch (e: CoreException) {
            loadError = e.message ?: e.kind
        }
    }
    val data = licenses
    val lines = remember(data) { data?.text?.lines().orEmpty() }
    var query by rememberSaveable { mutableStateOf("") }
    val matches = remember(lines, query) {
        val needle = query.trim()
        if (needle.isEmpty()) emptyList() else lines.indices.filter { lines[it].contains(needle, ignoreCase = true) }
    }
    var current by remember(matches) { mutableIntStateOf(-1) }
    val listState = rememberLazyListState()
    val highlight = MaterialTheme.colorScheme.secondaryContainer
    val jump: (Boolean) -> Unit = { forward ->
        if (matches.isNotEmpty()) {
            current = when {
                current < 0 -> if (forward) 0 else matches.size - 1
                forward -> (current + 1) % matches.size
                else -> (current - 1 + matches.size) % matches.size
            }
            val target = matches[current]
            scope.launch { listState.scrollToItem(maxOf(0, target - 3)) }
        }
    }

    SubPageScaffold(title = "Lizenzen der Rust-Bibliotheken", onBack = onBack) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            LoadingBar(data == null && loadError == null)
            val error = loadError
            when {
                error != null -> ErrorCard(
                    error,
                    modifier = Modifier.padding(16.dp),
                    title = "Lizenzen nicht geladen",
                    actionLabel = "Erneut",
                    onAction = { reloadKey++ },
                )
                data == null -> Unit
                !data.embedded -> Text(
                    data.text,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.padding(16.dp),
                )
                else -> {
                    Text(
                        "${data.crates.size} Rust-Bibliotheken · SHA-256 ${data.sha256.orEmpty().take(16)}…",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    )
                    OutlinedTextField(
                        value = query,
                        onValueChange = { query = it },
                        singleLine = true,
                        placeholder = { Text("Suchen…") },
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp),
                    )
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(horizontal = 16.dp)) {
                        Text(
                            when {
                                query.isBlank() -> ""
                                matches.isEmpty() -> "Keine Treffer"
                                current >= 0 -> "Treffer ${current + 1} von ${matches.size}"
                                else -> "${matches.size} Treffer"
                            },
                            style = MaterialTheme.typography.bodySmall,
                            modifier = Modifier.weight(1f),
                        )
                        TextButton(onClick = { jump(false) }, enabled = matches.isNotEmpty()) { Text("Vorheriger") }
                        TextButton(onClick = { jump(true) }, enabled = matches.isNotEmpty()) { Text("Nächster") }
                    }
                    val highlighted = matches.getOrNull(current)
                    val lineStyle = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace)
                    LazyColumn(state = listState, modifier = Modifier.fillMaxSize()) {
                        items(lines.size) { index ->
                            Text(
                                lines[index],
                                style = lineStyle,
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .background(if (index == highlighted) highlight else Color.Transparent)
                                    .padding(horizontal = 16.dp),
                            )
                        }
                    }
                }
            }
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
