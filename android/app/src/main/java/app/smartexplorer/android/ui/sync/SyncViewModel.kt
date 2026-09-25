package app.smartexplorer.android.ui.sync

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.smartexplorer.android.api.BgStatus
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.api.SyncApi.displayText
import app.smartexplorer.android.api.SyncApi.failureText
import app.smartexplorer.android.api.SyncApi.resultAs
import app.smartexplorer.android.api.SyncConflict
import app.smartexplorer.android.api.SyncJob
import app.smartexplorer.android.api.SyncOptions
import app.smartexplorer.android.api.SyncRunResult
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreEvent
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.Snackbars
import kotlinx.coroutines.launch

/** Sub-pages of the Sync tab. */
internal sealed interface SyncPage {
    data object Jobs : SyncPage

    data class Editor(val draft: JobDraft) : SyncPage

    data class Conflicts(val session: ConflictSession) : SyncPage

    data class Merge(val merge: MergeSession) : SyncPage

    data object Background : SyncPage
}

/**
 * State of the Sync tab (spec F15/F16), scoped to the activity so switching tabs keeps an open
 * editor or conflict list. All core calls run in [viewModelScope].
 */
internal class SyncViewModel : ViewModel() {
    var jobs by mutableStateOf<List<SyncJob>>(emptyList())
        private set
    var loaded by mutableStateOf(false)
        private set
    var loading by mutableStateOf(false)
        private set
    var loadError by mutableStateOf<String?>(null)
        private set
    var status by mutableStateOf<BgStatus?>(null)
        private set
    var page by mutableStateOf<SyncPage>(SyncPage.Jobs)
        private set

    /** Text for the [Details] dialog, `null` when closed. */
    var details by mutableStateOf<String?>(null)

    /** Job id → task id of runs started here until `sync.jobs` reports `runningTask`. */
    val startedRuns = mutableStateMapOf<String, String>()

    private var options: SyncOptions? = null

    init {
        viewModelScope.launch {
            Core.events.collect { event -> if (event is CoreEvent.Jobs) loadJobs() }
        }
    }

    /** [Aktualisieren]: jobs and background status. */
    fun refresh() {
        reloadJobs()
        viewModelScope.launch { loadStatus() }
    }

    /** Reloads `sync.jobs` (whenever the job list is shown). */
    fun reloadJobs() {
        viewModelScope.launch { loadJobs() }
    }

    /** `bg.status` for the header card; failures only leave the old status. */
    suspend fun loadStatus() {
        status = try {
            SyncApi.status()
        } catch (e: CoreException) {
            status
        }
    }

    private suspend fun loadJobs() {
        loading = true
        try {
            jobs = SyncApi.jobs()
            loadError = null
            loaded = true
        } catch (e: CoreException) {
            loadError = e.displayText()
        } finally {
            loading = false
        }
    }

    fun back() {
        when (val current = page) {
            is SyncPage.Merge -> page = SyncPage.Conflicts(current.merge.conflicts)
            is SyncPage.Conflicts -> closeConflicts(current.session)
            else -> page = SyncPage.Jobs
        }
    }

    fun showBackground() {
        page = SyncPage.Background
    }

    // ── Jobs ────────────────────────────────────────────────────────────────

    fun runNow(job: SyncJob) {
        viewModelScope.launch {
            val taskId = try {
                SyncApi.run(job.id)
            } catch (e: CoreException) {
                Snackbars.show("„${job.name}“ nicht gestartet: ${e.displayText()}")
                return@launch
            }
            startedRuns[job.id] = taskId
            val end = try {
                SyncApi.awaitTask(taskId)
            } catch (e: CoreException) {
                startedRuns.remove(job.id)
                return@launch
            }
            startedRuns.remove(job.id)
            loadJobs()
            reportRun(job, end.state, end.resultAs<SyncRunResult>(), end.failureText())
        }
    }

    private fun reportRun(job: SyncJob, state: String, result: SyncRunResult?, failure: String) {
        val name = job.name.ifBlank { "Sync-Job" }
        when {
            state == "done" && result != null && result.conflicts > 0 ->
                Snackbars.show("$name: ${result.conflicts} Konflikte", "Konflikte") { openConflicts(job) }
            state == "done" -> Snackbars.show("$name: ${result?.summary?.ifBlank { null } ?: "fertig"}")
            state == "canceled" -> Snackbars.show("$name: abgebrochen")
            else -> Snackbars.show("$name fehlgeschlagen", "Details") { details = failure }
        }
    }

    fun setEnabled(job: SyncJob, enabled: Boolean) {
        viewModelScope.launch {
            try {
                val updated = SyncApi.setEnabled(job.id, enabled)
                jobs = jobs.map { if (it.id == updated.id) updated else it }
            } catch (e: CoreException) {
                Snackbars.show("Nicht geändert: ${e.displayText()}")
            }
        }
    }

    fun delete(job: SyncJob) {
        viewModelScope.launch {
            try {
                SyncApi.delete(job.id)
                jobs = jobs.filterNot { it.id == job.id }
                Snackbars.show("„${job.name}“ gelöscht")
            } catch (e: CoreException) {
                Snackbars.show("Nicht gelöscht: ${e.displayText()}")
            }
        }
    }

    // ── Editor ──────────────────────────────────────────────────────────────

    /** Opens the editor for [job], or for a new job (optionally prefilled from a mirror run). */
    fun openEditor(job: SyncJob? = null, prefill: JobPrefill? = null) {
        viewModelScope.launch {
            val opts = loadOptions() ?: return@launch
            val start = when {
                job != null -> job
                prefill != null -> prefilled(opts, prefill)
                else -> opts.defaults.copy(id = "")
            }
            page = SyncPage.Editor(JobDraft(start, opts))
        }
    }

    /** Mirror → job: one-way A→B that never deletes in the target, like the mirror run. */
    private fun prefilled(opts: SyncOptions, prefill: JobPrefill): SyncJob {
        val base = opts.defaults.copy(id = "", source = prefill.source, target = prefill.target)
        val oneWay = opts.directions.any { it.value == SyncJob.DIRECTION_A_TO_B }
        val noDelete = opts.deletePolicies.any { it.value == SyncJob.DELETE_NONE }
        return base.copy(
            direction = if (oneWay) SyncJob.DIRECTION_A_TO_B else base.direction,
            deletePolicy = if (noDelete) SyncJob.DELETE_NONE else base.deletePolicy,
        )
    }

    private suspend fun loadOptions(): SyncOptions? {
        options?.let { return it }
        return try {
            SyncApi.options().also { options = it }
        } catch (e: CoreException) {
            Snackbars.show("Editor nicht verfügbar: ${e.displayText()}")
            null
        }
    }

    /** Local checks, then the desktop validation (`sync.validate`), then `sync.save`. */
    fun save(draft: JobDraft) {
        if (draft.saving) return
        val (job, problems) = draft.build()
        draft.errors = problems
        if (problems.isNotEmpty()) return
        draft.saving = true
        viewModelScope.launch {
            try {
                val errors = SyncApi.validate(job)
                if (errors.isNotEmpty()) {
                    draft.errors = errors
                    return@launch
                }
                val saved = SyncApi.save(job)
                jobs = if (jobs.any { it.id == saved.id }) jobs.map { if (it.id == saved.id) saved else it } else jobs + saved
                if ((page as? SyncPage.Editor)?.draft === draft) page = SyncPage.Jobs
                Snackbars.show("„${saved.name}“ gespeichert")
            } catch (e: CoreException) {
                draft.errors = mapOf("save" to "Nicht gespeichert: ${e.displayText()}")
            } finally {
                draft.saving = false
            }
        }
    }

    // ── Conflicts ───────────────────────────────────────────────────────────

    fun openConflicts(job: SyncJob) {
        val session = ConflictSession(job, viewModelScope) { text -> details = text }
        page = SyncPage.Conflicts(session)
        session.load()
    }

    fun openConflictsById(jobId: String) {
        viewModelScope.launch {
            if (!loaded) loadJobs()
            val job = jobs.firstOrNull { it.id == jobId }
            if (job == null) {
                Snackbars.show("Sync-Job nicht gefunden")
            } else {
                openConflicts(job)
            }
        }
    }

    /** Leaving the list stores pending resolutions; on failure the page stays with [Erneut versuchen]. */
    fun closeConflicts(session: ConflictSession) {
        session.finish { page = SyncPage.Jobs }
    }

    /** Closes the conflict list without storing pending resolutions (after a failed save). */
    fun abandonConflicts() {
        page = SyncPage.Jobs
    }

    fun openMerge(session: ConflictSession, conflict: SyncConflict) {
        val merge = MergeSession(session, conflict, viewModelScope) { page = SyncPage.Conflicts(session) }
        page = SyncPage.Merge(merge)
        merge.load()
    }
}
