package app.smartexplorer.android.ui.sync

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.api.SyncApi.displayText
import app.smartexplorer.android.api.SyncApi.failureText
import app.smartexplorer.android.api.SyncConflict
import app.smartexplorer.android.api.SyncJob
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.Snackbars
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

/**
 * Conflict list of one two-way job (spec F16, api.md `sync.conflicts` …). Without a stored
 * context (`available == false`) the user starts a dry run (`sync.checkConflicts`). Resolved
 * and skipped entries disappear; [finish] stores the baseline when leaving.
 */
@Stable
internal class ConflictSession(
    val job: SyncJob,
    private val scope: CoroutineScope,
    private val onDetails: (String) -> Unit,
) {
    var loading by mutableStateOf(false)
        private set
    var available by mutableStateOf(true)
        private set
    var items by mutableStateOf<List<SyncConflict>>(emptyList())
        private set
    var loadError by mutableStateOf<String?>(null)
        private set

    /** Task id of a running dry run. */
    var checkTaskId by mutableStateOf<String?>(null)
        private set

    /** Keys of entries that are being resolved right now. */
    val busy = mutableStateMapOf<String, Boolean>()
    var bulk by mutableStateOf(false)
        private set
    var finishing by mutableStateOf(false)
        private set

    /** Error of the last baseline save; the page offers [Erneut versuchen]. */
    var finishError by mutableStateOf<String?>(null)
        private set

    /** Something was resolved, merged or skipped since the page opened. */
    private var changed = false

    val working: Boolean get() = bulk || finishing || checkTaskId != null || busy.isNotEmpty()

    fun load() {
        scope.launch { reload() }
    }

    /** Opens the [Details] dialog of the Sync tab. */
    fun showDetails(text: String) = onDetails(text)

    private suspend fun reload() {
        loading = true
        try {
            val result = SyncApi.conflicts(job.id)
            available = result.available
            items = result.items
            loadError = null
        } catch (e: CoreException) {
            loadError = e.displayText()
        } finally {
            loading = false
        }
    }

    /** Dry run ("Konflikte prüfen"); the list reloads when it ends. */
    fun check() {
        if (checkTaskId != null) return
        scope.launch {
            val id = try {
                SyncApi.checkConflicts(job.id)
            } catch (e: CoreException) {
                Snackbars.show("Prüfung nicht gestartet: ${e.displayText()}")
                return@launch
            }
            checkTaskId = id
            try {
                val end = SyncApi.awaitTask(id)
                if (end.state == "failed") {
                    Snackbars.show("Prüfung fehlgeschlagen", "Details") { showDetails(end.failureText()) }
                }
            } catch (e: CoreException) {
                Snackbars.show("Prüfung: ${e.displayText()}")
            } finally {
                checkTaskId = null
            }
            reload()
        }
    }

    fun cancelCheck() {
        val id = checkTaskId ?: return
        scope.launch {
            try {
                SyncApi.cancelTask(id)
            } catch (e: CoreException) {
                Snackbars.show("Nicht abgebrochen: ${e.displayText()}")
            }
        }
    }

    /** [choice] is `"a"` or `"b"`. */
    fun resolve(item: SyncConflict, choice: String) {
        scope.launch {
            val failure = resolveOne(item, choice)
            if (failure != null) Snackbars.show("${fileName(item)}: nicht gelöst", "Details") { showDetails(failure) }
            reload()
        }
    }

    /** [Alle A] / [Alle B]: one entry after the other; failures are summed up at the end. */
    fun resolveAll(choice: String) {
        if (bulk) return
        bulk = true
        scope.launch {
            val failures = mutableListOf<String>()
            try {
                for (item in items.toList()) {
                    resolveOne(item, choice)?.let { failures += "${item.path}: $it" }
                }
            } finally {
                bulk = false
            }
            if (failures.isNotEmpty()) {
                Snackbars.show("${failures.size} Konflikte nicht gelöst", "Details") { showDetails(failures.joinToString("\n")) }
            }
            reload()
        }
    }

    /** `null` on success, else the failure text. */
    private suspend fun resolveOne(item: SyncConflict, choice: String): String? {
        busy[item.key] = true
        return try {
            val end = SyncApi.awaitTask(SyncApi.resolve(job.id, item.cid, choice))
            if (end.state == "done") {
                markResolved(item)
                null
            } else {
                end.failureText()
            }
        } catch (e: CoreException) {
            e.displayText()
        } finally {
            busy.remove(item.key)
        }
    }

    /** Skips the entry for this session only (like the desktop). */
    fun skip(item: SyncConflict) {
        scope.launch {
            try {
                SyncApi.skip(job.id, item.cid)
                markResolved(item)
            } catch (e: CoreException) {
                Snackbars.show("${fileName(item)}: ${e.displayText()}")
            }
        }
    }

    /** Called by the merge view after it wrote the result. */
    fun markResolved(item: SyncConflict) {
        changed = true
        items = items.filterNot { it.key == item.key }
    }

    /** Stores pending resolutions (`sync.finishConflicts`), then [onClosed]; keeps the page on failure. */
    fun finish(onClosed: () -> Unit) {
        if (!changed) {
            onClosed()
            return
        }
        if (finishing) return
        finishing = true
        scope.launch {
            try {
                SyncApi.finishConflicts(job.id)
                changed = false
                finishError = null
                onClosed()
            } catch (e: CoreException) {
                finishError = e.displayText()
            } finally {
                finishing = false
            }
        }
    }

    companion object {
        fun fileName(item: SyncConflict): String = item.path.substringAfterLast('/').ifBlank { item.path }
    }
}
