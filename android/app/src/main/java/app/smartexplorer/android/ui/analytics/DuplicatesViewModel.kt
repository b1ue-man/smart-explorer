package app.smartexplorer.android.ui.analytics

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.smartexplorer.android.api.AnalyzeApi
import app.smartexplorer.android.api.DuplicateGroup
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.api.ReclaimSummary
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.Snackbars
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Duplicate search (spec F20, B5): scan, all groups of equal content, the search totals (checked
 * files, candidates, unreadable paths, protected areas, an early stop), selection of copies (never
 * the last copy of a group) and moving them to the trash. Places without a trash only show the groups.
 */
internal class DuplicatesViewModel : ViewModel() {
    var location by mutableStateOf<String?>(null)
    var minSize by mutableStateOf(DEFAULT_MIN_SIZE)
    var phase by mutableStateOf(ScanPhase.Setup)
        private set
    var taskId by mutableStateOf<String?>(null)
        private set
    var groups by mutableStateOf<List<DuplicateGroup>>(emptyList())
        private set
    var selected by mutableStateOf<Set<String>>(emptySet())
        private set

    /** Totals of the last search; `null` when the core could not report them. */
    var summary by mutableStateOf<ReclaimSummary?>(null)
        private set

    /** Minimum size the shown result was searched with. */
    var searchedMinSize by mutableStateOf(DEFAULT_MIN_SIZE)
        private set

    /**
     * Locations that name more than one listed copy (equal Google Drive names in one folder): a
     * delete by location cannot pick one of them, so they are never selectable.
     */
    var ambiguous by mutableStateOf<Set<String>>(emptySet())
        private set
    var error by mutableStateOf<String?>(null)
        private set

    /** The place has no trash (`fs.delete` answered `unsupported`): groups are shown only. */
    var trashUnsupported by mutableStateOf(false)
        private set
    var deleting by mutableStateOf(false)
        private set

    private var scanJob: Job? = null

    fun preselect(current: String?) {
        if (phase == ScanPhase.Setup && location == null) location = current
    }

    fun start() {
        val target = location ?: return
        if (phase == ScanPhase.Scanning) cancel()
        // The page shows one result: the former one is freed at once.
        taskId?.let { releaseResult(it) }
        scanJob?.cancel()
        phase = ScanPhase.Scanning
        error = null
        groups = emptyList()
        selected = emptySet()
        ambiguous = emptySet()
        summary = null
        trashUnsupported = false
        taskId = null
        val threshold = minSize
        scanJob = viewModelScope.launch {
            try {
                val id = AnalyzeApi.reclaimStart(target, threshold)
                taskId = id
                val task = FilesApi.awaitTask(id)
                when (task.state) {
                    "done" -> {
                        val all = AnalyzeApi.reclaimGroups(id)
                        summary = try {
                            AnalyzeApi.reclaimSummary(id)
                        } catch (e: CoreException) {
                            null
                        }
                        // All groups now (no 200 cap): order and duplicate-location check off the main thread.
                        val (found, shared) = withContext(Dispatchers.Default) { arrange(all) }
                        ambiguous = shared
                        groups = found
                        searchedMinSize = threshold
                        phase = ScanPhase.Result
                    }
                    "canceled" -> {
                        phase = ScanPhase.Setup
                        Snackbars.show("Suche abgebrochen")
                    }
                    else -> {
                        phase = ScanPhase.Setup
                        error = task.message?.takeIf { it.isNotBlank() } ?: "Suche fehlgeschlagen"
                    }
                }
            } catch (e: CoreException) {
                phase = ScanPhase.Setup
                error = e.message ?: e.kind
            }
        }
    }

    fun cancel() {
        val id = taskId ?: return
        viewModelScope.launch {
            try {
                FilesApi.cancelTask(id)
            } catch (e: CoreException) {
                Snackbars.show("Nicht abgebrochen: ${e.message ?: e.kind}")
            }
        }
    }

    fun backToSetup() {
        if (phase == ScanPhase.Scanning) cancel()
        taskId?.let { releaseResult(it) }
        taskId = null
        scanJob?.cancel()
        phase = ScanPhase.Setup
        groups = emptyList()
        selected = emptySet()
        ambiguous = emptySet()
        summary = null
    }

    /** Selects or deselects one copy; the last unselected copy of a group stays. */
    fun toggle(group: DuplicateGroup, location: String) {
        if (location in ambiguous) {
            Snackbars.show("Gleichnamige Dateien am selben Ort lassen sich nicht einzeln löschen.")
            return
        }
        if (location in selected) {
            selected = selected - location
            return
        }
        if (group.items.count { it.location !in selected } <= 1) {
            Snackbars.show("Mindestens eine Datei je Gruppe bleibt erhalten.")
            return
        }
        selected = selected + location
    }

    /** [Kopien automatisch auswählen]: all but the oldest copy of every group (never an [ambiguous] one). */
    fun autoSelect() {
        selected = groups.flatMap { group ->
            val keep = group.items.minByOrNull { it.mtimeMs } ?: return@flatMap emptyList()
            group.items.filter { it !== keep && it.location != keep.location && it.location !in ambiguous }.map { it.location }
        }.toSet()
    }

    /** [In den Papierkorb]: never permanent; failures keep their entries in the list. */
    fun trashSelected() {
        val targets = selected.toList()
        if (targets.isEmpty() || deleting) return
        viewModelScope.launch {
            deleting = true
            try {
                val task = FilesApi.awaitTask(FilesApi.delete(targets, permanent = false))
                val failed = task.errors.map { it.path }.toSet()
                // A partly failed task names its failures; without names nothing is known to be moved.
                val removed = when {
                    task.state == "done" || (task.state == "failed" && failed.isNotEmpty()) -> targets.filterNot { it in failed }.toSet()
                    else -> emptySet()
                }
                groups = groups.map { group -> group.copy(items = group.items.filter { it.location !in removed }) }.filter { it.items.size > 1 }
                selected = selected - removed
                when {
                    task.state == "done" && failed.isEmpty() -> Snackbars.show("${removed.size} Dateien in den Papierkorb verschoben")
                    task.state == "canceled" -> Snackbars.show("Abgebrochen – für eine aktuelle Liste erneut suchen")
                    else -> Snackbars.show(
                        "${removed.size} verschoben, ${targets.size - removed.size} nicht: " +
                            (task.message ?: task.errors.firstOrNull()?.message ?: "Fehler"),
                    )
                }
            } catch (e: CoreException) {
                if (e.kind == "unsupported") {
                    trashUnsupported = true
                    selected = emptySet()
                } else {
                    Snackbars.show("Nicht gelöscht: ${e.message ?: e.kind}")
                }
            } finally {
                deleting = false
            }
        }
    }

    /** The activity is gone for good: its result is unreachable, so the core frees it. */
    override fun onCleared() {
        taskId?.let { releaseResult(it) }
        super.onCleared()
    }

    /**
     * Groups with more than one copy, most space held by copies first, and the locations that name
     * more than one listed copy ([ambiguous]).
     */
    private fun arrange(all: List<DuplicateGroup>): Pair<List<DuplicateGroup>, Set<String>> {
        val found = all.filter { it.items.size > 1 }.sortedByDescending { it.size * (it.items.size - 1) }
        val shared = found.flatMap { group -> group.items.map { it.location } }
            .groupingBy { it }.eachCount().filterValues { it > 1 }.keys
        return found to shared
    }

    companion object {
        const val DEFAULT_MIN_SIZE = 1L shl 20
        val MIN_SIZES = listOf(100L shl 10, 1L shl 20, 10L shl 20, 100L shl 20)
    }
}
