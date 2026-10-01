package app.smartexplorer.android.ui.analytics

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.smartexplorer.android.api.AnalyzeApi
import app.smartexplorer.android.api.AnalyzeChild
import app.smartexplorer.android.api.AnalyzeIssues
import app.smartexplorer.android.api.AnalyzeKind
import app.smartexplorer.android.api.AnalyzeNode
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.system.StorageStatsAccess
import app.smartexplorer.android.ui.common.Snackbars
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull

/** Page of a scan-based tool: choose a place, scan with progress, result. */
internal enum class ScanPhase { Setup, Scanning, Result }

/**
 * Storage analysis (spec F19, B2/B3/B6). Lives as long as the activity, so a running analysis and
 * its drill-down survive tab switches; the scan itself is a core task.
 */
internal class AnalysisViewModel : ViewModel() {
    var location by mutableStateOf<String?>(null)
    var phase by mutableStateOf(ScanPhase.Setup)
        private set
    var taskId by mutableStateOf<String?>(null)
        private set

    /** Names below the analysed root of the shown folder (empty = root). */
    var path by mutableStateOf<List<String>>(emptyList())
        private set
    var node by mutableStateOf<AnalyzeNode?>(null)
        private set
    var issues by mutableStateOf<AnalyzeIssues?>(null)
        private set
    var loadingNode by mutableStateOf(false)
        private set
    var error by mutableStateOf<String?>(null)
        private set

    /** Depth of [path] at which a folder Android locks was entered; `null` outside such folders. */
    var protectedFrom by mutableStateOf<Int?>(null)
        private set

    /** The result's tree contains `Android/data` of the primary volume (usage access would size it). */
    var appDataInTree by mutableStateOf(false)
        private set

    /** The result was made with Android's sizes of other apps (usage access was granted). */
    var appDataIncluded by mutableStateOf(false)
        private set

    /** Android's figures being gathered before the core task starts (shown instead of "Wird gestartet"). */
    var preparing by mutableStateOf<String?>(null)
        private set

    /** App of the app list whose breakdown is shown ([open] on an app row). */
    var appDetail by mutableStateOf<AnalyzeChild?>(null)
        private set

    /** The usage access card was hidden for this session. */
    var usageHintDismissed by mutableStateOf(false)
        private set

    /** Application context for the platform figures; set by [AnalysisScreen]. */
    private val appContext = MutableStateFlow<Context?>(null)
    private var scanJob: Job? = null
    private var nodeJob: Job? = null

    /** The shown folder is one Android locks (or lies below one). */
    val insideProtected: Boolean
        get() = protectedFrom != null

    fun attach(context: Context) {
        appContext.value = context.applicationContext
    }

    /** "Mehr → Speicheranalyse": suggest [current] (the place shown in "Dateien") on the setup page. */
    fun preselect(current: String?) {
        if (phase == ScanPhase.Setup && location == null) location = current
    }

    /** Request from another screen: analyse [target], right away when [start]. */
    fun request(target: String, start: Boolean) {
        location = target
        if (start) start() else backToSetup()
    }

    fun start() {
        val target = location ?: return
        val previous = taskId.takeIf { phase == ScanPhase.Scanning }
        scanJob?.cancel()
        nodeJob?.cancel()
        previous?.let { cancelTask(it) }
        phase = ScanPhase.Scanning
        error = null
        node = null
        issues = null
        path = emptyList()
        protectedFrom = null
        appDataInTree = false
        appDataIncluded = false
        appDetail = null
        preparing = null
        taskId = null
        scanJob = viewModelScope.launch {
            try {
                val figures = platformFigures(target)
                preparing = null
                appDataInTree = figures?.appDataInTree == true
                appDataIncluded = figures?.includesApps == true
                val id = startTask { AnalyzeApi.start(target, figures?.platform) }
                taskId = id
                val task = FilesApi.awaitTask(id)
                when (task.state) {
                    "done" -> {
                        // Before the tree, so an empty locked root shows as such right away.
                        issues = try {
                            AnalyzeApi.issues(id)
                        } catch (e: CoreException) {
                            null
                        }
                        phase = ScanPhase.Result
                        loadNode(emptyList(), protectedFrom = null)
                    }
                    "canceled" -> {
                        phase = ScanPhase.Setup
                        Snackbars.show("Analyse abgebrochen")
                    }
                    else -> {
                        phase = ScanPhase.Setup
                        error = task.message?.takeIf { it.isNotBlank() } ?: "Analyse fehlgeschlagen"
                    }
                }
            } catch (e: CoreException) {
                phase = ScanPhase.Setup
                error = e.message ?: e.kind
            }
        }
    }

    /** [Abbrechen]: cancels the core task; before it exists, the preparation itself. */
    fun cancel() {
        val id = taskId
        if (id != null) {
            cancelTask(id)
            return
        }
        if (phase != ScanPhase.Scanning) return
        scanJob?.cancel()
        preparing = null
        phase = ScanPhase.Setup
        Snackbars.show("Analyse abgebrochen")
    }

    /**
     * Drill down into folder [child] (or the app list); an app row shows its breakdown; estimates and
     * aggregated rows have nothing behind them.
     */
    fun open(child: AnalyzeChild) {
        if (child.rowKind == AnalyzeKind.App) {
            appDetail = child
            return
        }
        if (!child.opensFolder) return
        val target = path + child.name
        val lockedFrom = protectedFrom ?: target.size.takeIf { child.rowKind == AnalyzeKind.Protected }
        loadNode(target, lockedFrom)
    }

    /** One level up; `false` at the analysed root. */
    fun up(): Boolean {
        if (path.isEmpty()) return false
        val target = path.dropLast(1)
        loadNode(target, protectedFrom?.takeIf { target.size >= it })
        return true
    }

    /** [Anderer Ort]: back to the setup page, the result is dropped. */
    fun backToSetup() {
        if (phase == ScanPhase.Scanning) taskId?.let { cancelTask(it) }
        scanJob?.cancel()
        nodeJob?.cancel()
        phase = ScanPhase.Setup
        node = null
        issues = null
        path = emptyList()
        protectedFrom = null
        appDetail = null
        preparing = null
        error = null
    }

    fun dismissUsageHint() {
        usageHintDismissed = true
    }

    fun closeAppDetail() {
        appDetail = null
    }

    /**
     * Platform figures for a local root (B2, B6: with usage access also the installed apps of the
     * whole internal storage). Waits briefly for [attach]: a request from another screen starts
     * before this page is composed (one frame later); without a context the analysis runs without
     * figures rather than waiting longer.
     */
    private suspend fun platformFigures(target: String): StorageStatsAccess.Figures? {
        if (!StorageStatsAccess.isLocalPath(target)) return null
        val context = appContext.value
            ?: withTimeoutOrNull(CONTEXT_WAIT_MS) { appContext.filterNotNull().first() }
            ?: return null
        preparing = "Speicherwerte von Android werden ermittelt …"
        return StorageStatsAccess.figuresFor(context, target)
    }

    /**
     * Runs the core start call to completion even when [cancel] or a new start cancels this job
     * meanwhile; the task it created is then canceled instead of running on unseen.
     */
    private suspend fun startTask(call: suspend () -> String): String {
        var started: String? = null
        try {
            withContext(NonCancellable) { started = call() }
        } catch (e: CancellationException) {
            started?.let { cancelTask(it) }
            throw e
        }
        return started ?: throw CoreException("internal", "Analyse ohne Task-Kennung gestartet")
    }

    private fun loadNode(target: List<String>, protectedFrom: Int?) {
        val id = taskId ?: return
        nodeJob?.cancel()
        val job = viewModelScope.launch {
            loadingNode = true
            try {
                node = AnalyzeApi.node(id, target)
                path = target
                this@AnalysisViewModel.protectedFrom = protectedFrom
                appDetail = null
                error = null
            } catch (e: CoreException) {
                Snackbars.show("Ordner nicht geladen: ${e.message ?: e.kind}")
            }
        }
        nodeJob = job
        // Only the newest request ends the indicator (a canceled older one must not).
        job.invokeOnCompletion { if (nodeJob === job) loadingNode = false }
    }

    private fun cancelTask(id: String) {
        viewModelScope.launch {
            try {
                FilesApi.cancelTask(id)
            } catch (e: CoreException) {
                Snackbars.show("Nicht abgebrochen: ${e.message ?: e.kind}")
            }
        }
    }

    private companion object {
        /** Covers the first composition of the page after a request from another screen. */
        const val CONTEXT_WAIT_MS = 2_000L
    }
}
