package app.smartexplorer.android.ui.more

import androidx.activity.compose.BackHandler
import androidx.annotation.DrawableRes
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.ListItem
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import app.smartexplorer.android.R
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.MainTab
import app.smartexplorer.android.ui.NavRequest
import app.smartexplorer.android.ui.analytics.AnalysisScreen
import app.smartexplorer.android.ui.analytics.AnalysisViewModel
import app.smartexplorer.android.ui.analytics.DuplicatesScreen
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.connections.ConnectionsScreen
import app.smartexplorer.android.ui.files.FilesLocation
import app.smartexplorer.android.ui.settings.AboutScreen
import app.smartexplorer.android.ui.settings.ErrorLogScreen
import app.smartexplorer.android.ui.settings.SettingsFocus
import app.smartexplorer.android.ui.settings.SettingsScreen
import app.smartexplorer.android.ui.settings.UpdateCard
import app.smartexplorer.android.ui.transfers.TransferTasks
import app.smartexplorer.android.ui.transfers.TransfersSheet
import app.smartexplorer.android.ui.trash.TrashScreen
import app.smartexplorer.android.update.UpdateChecker
import app.smartexplorer.android.update.UpdateState
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Pages of the "Mehr" tab. */
internal enum class MorePage { Menu, Analysis, Duplicates, Connections, Trash, Settings, ErrorLog, About }

/**
 * Requests from other screens to open a page of "Mehr" for which no [NavRequest] exists (e.g.
 * "Dateien → ⋮ → Analysieren", "Verbindung hinzufügen"). [open] also switches to the tab.
 */
object MoreRequests {
    sealed interface Request {
        /** Storage analysis of [location], started right away. */
        data class Analyze(val location: String) : Request

        /** Connection list with the form for a new connection open. */
        data object AddConnection : Request

        data object Connections : Request

        data object ErrorLog : Request
    }

    private val state = MutableStateFlow<Request?>(null)
    val pending: StateFlow<Request?> = state.asStateFlow()

    fun open(request: Request) {
        state.value = request
        AppNav.send(NavRequest.SelectTab(MainTab.More))
    }

    /** Marks [request] handled; a newer one stays pending. */
    fun consume(request: Request) {
        state.compareAndSet(request, null)
    }
}

/**
 * Tab "Mehr" (spec C): Speicheranalyse · Duplikate finden · Verbindungen · Papierkorb ·
 * Übertragungen · Einstellungen · Fehlerprotokoll · Über, each a page with a back arrow. Handles
 * [NavRequest.ShowUpdate] and [NavRequest.ShowBackgroundSettings] (settings, scrolled to the
 * section) and [MoreRequests].
 */
@Composable
fun MoreScreen() {
    var page by rememberSaveable { mutableStateOf(MorePage.Menu) }
    // ErrorLog/About open from the menu and from the settings; back returns there.
    var origin by rememberSaveable { mutableStateOf(MorePage.Menu) }
    var settingsFocus by rememberSaveable { mutableStateOf<SettingsFocus?>(null) }
    var newConnection by rememberSaveable { mutableStateOf(false) }
    var showTransfers by rememberSaveable { mutableStateOf(false) }
    val analysis = viewModel { AnalysisViewModel() }
    val navPending by AppNav.pending.collectAsStateWithLifecycle()
    val morePending by MoreRequests.pending.collectAsStateWithLifecycle()

    LaunchedEffect(navPending) {
        val request = navPending ?: return@LaunchedEffect
        val focus = when (request) {
            NavRequest.ShowUpdate -> SettingsFocus.Updates
            NavRequest.ShowBackgroundSettings -> SettingsFocus.Background
            else -> return@LaunchedEffect
        }
        settingsFocus = focus
        page = MorePage.Settings
        AppNav.consume(request)
    }
    LaunchedEffect(morePending) {
        val request = morePending ?: return@LaunchedEffect
        when (request) {
            is MoreRequests.Request.Analyze -> {
                analysis.request(request.location, start = true)
                page = MorePage.Analysis
            }
            MoreRequests.Request.AddConnection -> {
                newConnection = true
                page = MorePage.Connections
            }
            MoreRequests.Request.Connections -> page = MorePage.Connections
            MoreRequests.Request.ErrorLog -> {
                origin = MorePage.Menu
                page = MorePage.ErrorLog
            }
        }
        MoreRequests.consume(request)
    }

    val back: () -> Unit = {
        page = if (page == MorePage.ErrorLog || page == MorePage.About) origin else MorePage.Menu
    }
    // Composed before the pages, so their own back handlers (drill-down, forms) take precedence.
    BackHandler(enabled = page != MorePage.Menu, onBack = back)

    when (page) {
        MorePage.Menu -> MoreMenu(
            onOpen = { target ->
                origin = MorePage.Menu
                if (target == MorePage.Analysis) analysis.preselect(FilesLocation.current.value)
                page = target
            },
            onShowTransfers = { showTransfers = true },
        )
        MorePage.Analysis -> AnalysisScreen(analysis, onClose = back)
        MorePage.Duplicates -> DuplicatesScreen(onBack = back)
        MorePage.Connections -> ConnectionsScreen(
            startWithNewForm = newConnection,
            onNewFormShown = { newConnection = false },
            onBack = back,
        )
        MorePage.Trash -> TrashScreen(onBack = back)
        MorePage.Settings -> SettingsScreen(
            focus = settingsFocus,
            onFocusHandled = { settingsFocus = null },
            onBack = back,
            onOpenErrorLog = {
                origin = MorePage.Settings
                page = MorePage.ErrorLog
            },
            onOpenAbout = {
                origin = MorePage.Settings
                page = MorePage.About
            },
        )
        MorePage.ErrorLog -> ErrorLogScreen(onBack = back)
        MorePage.About -> AboutScreen(onBack = back)
    }
    if (showTransfers) TransfersSheet(onDismiss = { showTransfers = false })
}

private data class MenuEntry(val page: MorePage?, val label: String, @DrawableRes val icon: Int)

private val MENU = listOf(
    MenuEntry(MorePage.Analysis, "Speicheranalyse", R.drawable.ic_analytics),
    MenuEntry(MorePage.Duplicates, "Duplikate finden", R.drawable.ic_duplicate),
    MenuEntry(MorePage.Connections, "Verbindungen", R.drawable.ic_cloud),
    MenuEntry(MorePage.Trash, "Papierkorb", R.drawable.ic_trash),
    // `null`: the transfers sheet, not a page.
    MenuEntry(null, "Übertragungen", R.drawable.ic_upload),
    MenuEntry(MorePage.Settings, "Einstellungen", R.drawable.ic_settings),
    MenuEntry(MorePage.ErrorLog, "Fehlerprotokoll", R.drawable.ic_warning),
    MenuEntry(MorePage.About, "Über", R.drawable.ic_info),
)

@Composable
private fun MoreMenu(onOpen: (MorePage) -> Unit, onShowTransfers: () -> Unit) {
    val update by UpdateChecker.state.collectAsStateWithLifecycle()
    val tasks by Core.tasks.collectAsStateWithLifecycle()
    val running = tasks.count { it.isActive && TransferTasks.isTransfer(it) }
    // Only a pending update earns space above the list (spec F22 "Update-Karte").
    val showUpdate = update is UpdateState.Available || update is UpdateState.Downloading || update is UpdateState.Ready
    SubPageScaffold(title = "Mehr", onBack = null) { padding ->
        LazyColumn(Modifier.fillMaxSize().padding(padding)) {
            if (showUpdate) {
                item(key = "update") { UpdateCard(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) }
            }
            items(MENU, key = { it.label }) { entry ->
                ListItem(
                    headlineContent = { Text(entry.label) },
                    supportingContent = if (entry.page == null && running > 0) {
                        { Text("$running laufen") }
                    } else {
                        null
                    },
                    leadingContent = { SeIcon(entry.icon, contentDescription = null) },
                    modifier = Modifier.clickable { entry.page?.let(onOpen) ?: onShowTransfers() },
                )
            }
        }
    }
}
