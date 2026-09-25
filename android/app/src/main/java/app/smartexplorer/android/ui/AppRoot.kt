package app.smartexplorer.android.ui

import android.app.Application
import androidx.activity.compose.BackHandler
import androidx.annotation.DrawableRes
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.adaptive.ExperimentalMaterial3AdaptiveApi
import androidx.compose.material3.adaptive.currentWindowAdaptiveInfo
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.window.core.layout.WindowSizeClass
import app.smartexplorer.android.R
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreEvent
import app.smartexplorer.android.core.CoreState
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.ui.common.AppSnackbarHost
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.files.FilesScreen
import app.smartexplorer.android.ui.more.MoreRequests
import app.smartexplorer.android.ui.more.MoreScreen
import app.smartexplorer.android.ui.onboarding.OnboardingScreen
import app.smartexplorer.android.ui.onboarding.StartupErrorScreen
import app.smartexplorer.android.ui.onboarding.StartupScreen
import app.smartexplorer.android.ui.share.ShareScreen
import app.smartexplorer.android.ui.sync.SyncScreen
import kotlinx.coroutines.flow.onSubscription

/** Top level: setup page on first start, core start/failure, then the four main tabs. */
@Composable
fun AppRoot() {
    val snackbarHostState = remember { SnackbarHostState() }
    val onboardingDone by AppPrefs.onboardingDone.collectAsStateWithLifecycle()
    val coreState by Core.ready.collectAsStateWithLifecycle()
    val app = LocalContext.current.applicationContext as Application
    val state = coreState
    when {
        !onboardingDone -> WithSnackbars(snackbarHostState) {
            OnboardingScreen(onDone = { AppPrefs.setOnboardingDone(true) })
        }
        state is CoreState.Failed -> WithSnackbars(snackbarHostState) {
            StartupErrorScreen(state.message, onRetry = { Core.start(app) })
        }
        state == CoreState.Ready -> MainShell(snackbarHostState)
        else -> StartupScreen()
    }
}

@Composable
private fun WithSnackbars(hostState: SnackbarHostState, content: @Composable () -> Unit) {
    Box(Modifier.fillMaxSize()) {
        content()
        AppSnackbarHost(hostState, Modifier.align(Alignment.BottomCenter).navigationBarsPadding())
    }
}

private data class TabItem(val tab: MainTab, val label: String, @DrawableRes val icon: Int)

private val TABS = listOf(
    TabItem(MainTab.Files, "Dateien", R.drawable.ic_folder),
    TabItem(MainTab.Sync, "Sync", R.drawable.ic_sync),
    TabItem(MainTab.Share, "Teilen", R.drawable.ic_share),
    TabItem(MainTab.More, "Mehr", R.drawable.ic_more_vert),
)

/** Navigation bar below 840 dp width, navigation rail from 840 dp (spec C). */
@OptIn(ExperimentalLayoutApi::class, ExperimentalMaterial3AdaptiveApi::class)
@Composable
private fun MainShell(snackbarHostState: SnackbarHostState) {
    var tab by rememberSaveable { mutableStateOf(MainTab.Files) }
    var shareRequests by remember { mutableIntStateOf(0) }
    val wide = currentWindowAdaptiveInfo().windowSizeClass
        .isWidthAtLeastBreakpoint(WindowSizeClass.WIDTH_DP_EXPANDED_LOWER_BOUND)

    LaunchedEffect(Unit) {
        // Flush only once subscribed, so queued requests reach this collector.
        AppNav.requests.onSubscription { AppNav.flushQueued() }.collect { request ->
            tab = AppNav.tabFor(request)
            if (request is NavRequest.SelectTab) AppNav.consume(request)
        }
    }
    LaunchedEffect(Unit) {
        Core.events.collect { event ->
            if (event is CoreEvent.Error) {
                Snackbars.show(event.message, "Details") { MoreRequests.open(MoreRequests.Request.ErrorLog) }
            } else if (event is CoreEvent.ShareRequest && tab != MainTab.Share) {
                shareRequests = event.count
            }
        }
    }
    LaunchedEffect(tab) { if (tab == MainTab.Share) shareRequests = 0 }
    BackHandler(enabled = tab != MainTab.Files) { tab = MainTab.Files }

    Scaffold(
        bottomBar = {
            if (!wide) {
                NavigationBar {
                    TABS.forEach { item ->
                        NavigationBarItem(
                            selected = tab == item.tab,
                            onClick = { tab = item.tab },
                            icon = { TabIcon(item, shareRequests) },
                            label = { Text(item.label) },
                        )
                    }
                }
            }
        },
        snackbarHost = { AppSnackbarHost(snackbarHostState) },
        // Each tab screen handles its own insets (top bar, lists); only the bar height is consumed here.
        contentWindowInsets = WindowInsets(0, 0, 0, 0),
    ) { padding ->
        Row(Modifier.fillMaxSize().padding(padding).consumeWindowInsets(padding)) {
            if (wide) {
                NavigationRail {
                    TABS.forEach { item ->
                        NavigationRailItem(
                            selected = tab == item.tab,
                            onClick = { tab = item.tab },
                            icon = { TabIcon(item, shareRequests) },
                            label = { Text(item.label) },
                        )
                    }
                }
            }
            val contentModifier = Modifier.weight(1f).fillMaxHeight()
            Box(
                if (wide) {
                    contentModifier.consumeWindowInsets(WindowInsets.safeDrawing.only(WindowInsetsSides.Start))
                } else {
                    contentModifier
                },
            ) {
                when (tab) {
                    MainTab.Files -> FilesScreen()
                    MainTab.Sync -> SyncScreen()
                    MainTab.Share -> ShareScreen()
                    MainTab.More -> MoreScreen()
                }
            }
        }
    }
}

@Composable
private fun TabIcon(item: TabItem, shareRequests: Int) {
    if (item.tab == MainTab.Share && shareRequests > 0) {
        BadgedBox(badge = {
            Badge(Modifier.semantics { contentDescription = "$shareRequests neue Anfragen" }) {
                Text(shareRequests.toString())
            }
        }) { SeIcon(item.icon, contentDescription = null) }
    } else {
        SeIcon(item.icon, contentDescription = null)
    }
}
