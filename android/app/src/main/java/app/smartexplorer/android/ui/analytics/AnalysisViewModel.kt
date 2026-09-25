package app.smartexplorer.android.ui.analytics

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.smartexplorer.android.api.AnalyzeApi
import app.smartexplorer.android.api.AnalyzeChild
import app.smartexplorer.android.api.AnalyzeIssues
import app.smartexplorer.android.api.AnalyzeNode
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.Snackbars
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/** Page of a scan-based tool: choose a place, scan with progress, result. */
internal enum class ScanPhase { Setup, Scanning, Result }

/**
 * Storage analysis (spec F19). Lives as long as the activity, so a running analysis and its
 * drill-down survive tab switches; the scan itself is a core task.
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

    private var scanJob: Job? = null
    private var nodeJob: Job? = null

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
        taskId = null
        scanJob = viewModelScope.launch {
            try {
                val id = AnalyzeApi.start(target)
                taskId = id
                val task = FilesApi.awaitTask(id)
                when (task.state) {
                    "done" -> {
                        phase = ScanPhase.Result
                        loadNode(emptyList())
                        issues = try {
                            AnalyzeApi.issues(id)
                        } catch (e: CoreException) {
                            null
                        }
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

    fun cancel() {
        taskId?.let { cancelTask(it) }
    }

    /** Drill down into folder [child]. */
    fun open(child: AnalyzeChild) {
        if (child.isDir) loadNode(path + child.name)
    }

    /** One level up; `false` at the analysed root. */
    fun up(): Boolean {
        if (path.isEmpty()) return false
        loadNode(path.dropLast(1))
        return true
    }

    /** [Anderer Ort]: back to the setup page, the result is dropped. */
    fun backToSetup() {
        if (phase == ScanPhase.Scanning) cancel()
        scanJob?.cancel()
        nodeJob?.cancel()
        phase = ScanPhase.Setup
        node = null
        issues = null
        path = emptyList()
        error = null
    }

    private fun loadNode(target: List<String>) {
        val id = taskId ?: return
        nodeJob?.cancel()
        val job = viewModelScope.launch {
            loadingNode = true
            try {
                node = AnalyzeApi.node(id, target)
                path = target
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
}
