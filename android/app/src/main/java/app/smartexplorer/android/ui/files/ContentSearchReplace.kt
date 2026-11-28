package app.smartexplorer.android.ui.files

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.smartexplorer.android.api.ContentSearchQuery
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.api.ReplaceApi
import app.smartexplorer.android.api.ReplaceHit
import app.smartexplorer.android.api.ReplacePoll
import app.smartexplorer.android.api.ReplaceSummaryDto
import app.smartexplorer.android.core.CoreException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

// "In Dateien ersetzen" (api.md §4.14, plan docs/plaene/2026-11-28-ersetzen-masken-shell, part C):
// a dry run counts the places, the confirmation names folder, filter, places and files with
// "Abbrechen" as the emphasized choice (Notepad++ preselects it), the run's outcome shows its totals,
// the skipped files with their reasons and "Rückgängig".

/** One replacement: every match below [location] ([hits] null) or the chosen hit lines. */
internal data class ReplaceJob(
    val location: String,
    val query: ContentSearchQuery,
    val replacement: String,
    val hits: List<ReplaceHit>?,
) {
    /** Places the chosen hit lines name (a cut line counts once); null for every match. */
    val chosenPlaces: Int?
        get() = hits?.sumOf { maxOf(1, it.ranges.size) }
}

/** Where the confirmation stands. */
internal sealed interface ReplaceStage {
    /** Started from the result: "Ersetzen durch" is asked first. */
    data class Asking(val text: String) : ReplaceStage

    /** The dry run counts. */
    data class Counting(val files: Long, val replacements: Long) : ReplaceStage

    data class Ready(val filesChanged: Long, val replacements: Long, val skipped: List<SkippedFile>) : ReplaceStage

    data class Failed(val message: String) : ReplaceStage
}

internal data class ReplaceConfirm(val job: ReplaceJob, val stage: ReplaceStage)

private fun places(count: Long) = if (count == 1L) "Stelle" else "Stellen"

private fun filesWord(count: Long) = if (count == 1L) "Datei" else "Dateien"

private fun replacementsWord(count: Long) = if (count == 1L) "Ersetzung" else "Ersetzungen"

/** The text of the confirmation, one line each (as on the desktop). */
internal fun confirmLines(job: ReplaceJob, ready: ReplaceStage.Ready): List<String> = buildList {
    add(if (job.hits == null) "Wirklich alle Vorkommen ersetzen in:" else "Wirklich die ausgewählten Treffer ersetzen in:")
    add(job.location)
    add("Für Dateitypen: ${job.query.filters.trim().ifEmpty { "*.*" }}")
    if (job.query.respectIgnoreFiles) add("Ignorierdateien (.gitignore) werden beachtet.")
    val counts = "${ready.replacements} ${places(ready.replacements)} in ${ready.filesChanged} ${filesWord(ready.filesChanged)}"
    add(if (job.replacement.isEmpty()) "$counts löschen (Ersetzen durch nichts)?" else "$counts durch „${job.replacement}“ ersetzen?")
    if (ready.skipped.isNotEmpty()) add("${ready.skipped.size} übersprungen (Gründe unten).")
    add("Vor dem Überschreiben wird jede Datei gesichert; „Rückgängig“ stellt sie wieder her.")
}

/** "Rückgängig" of a run. */
internal sealed interface UndoStage {
    data object Unavailable : UndoStage
    data class Available(val undoId: String) : UndoStage
    data object Running : UndoStage
    data object Done : UndoStage

    /** Restored, except [left] (changed since the run), with the reasons. */
    data class Partly(val left: List<SkippedFile>) : UndoStage
    data class Failed(val message: String) : UndoStage
}

/** The outcome of one run, built from the `search.content.replace.poll` answers. */
internal class ReplaceOutcome(val job: ReplaceJob) {
    private val files = LinkedHashMap<String, Long>()
    private val skippedFiles = ArrayList<SkippedFile>()

    var summary: ReplaceSummaryDto? = null
        private set
    var searched = 0L
        private set
    var done = false
        private set
    var canceled = false
        private set
    var error: String? = null
        private set
    var undo: UndoStage = UndoStage.Unavailable
        internal set

    /** (relative path, replacements) of the changed files in arrival order. */
    val changed: List<Pair<String, Long>>
        get() = files.entries.map { it.key to it.value }

    val skipped: List<SkippedFile>
        get() = skippedFiles

    val replacements: Long
        get() = summary?.replacements ?: files.values.sum()

    fun apply(poll: ReplacePoll) {
        for (event in poll.events) {
            when (event.type) {
                "file" -> files[event.rel] = (files[event.rel] ?: 0L) + event.replacements
                "skipped" -> skippedFiles += SkippedFile(event.rel, event.location, event.reason ?: "übersprungen")
            }
        }
        searched = maxOf(searched, poll.summary?.filesSearched ?: poll.files)
        poll.summary?.let {
            summary = it
            undo = it.undoId?.let(UndoStage::Available) ?: UndoStage.Unavailable
        }
        if (poll.done) done = true
        if (poll.canceled) canceled = true
        if (poll.error != null) error = poll.error
    }

    /** `Ersetzen „foo“ durch „bar“ (3 Ersetzungen in 2 Dateien von 10 durchsuchten, 1 übersprungen)`. */
    fun header(): String {
        val what = if (job.hits == null) "Ersetzen" else "Ersetzen (Auswahl)"
        val head = "$what „${job.query.pattern}“ durch „${job.replacement}“"
        val count = files.size.toLong()
        val counts = "$replacements ${replacementsWord(replacements)} in $count ${filesWord(count)}"
        val skippedText = "${skippedFiles.size} übersprungen"
        val body = when {
            error != null -> "fehlgeschlagen nach $counts, $skippedText: $error"
            canceled -> "(abgebrochen: $counts, $skippedText)"
            done -> "($counts von $searched durchsuchten, $skippedText)"
            else -> "(läuft: $counts, $searched durchsucht)"
        }
        val undoText = when (val stage = undo) {
            UndoStage.Running -> " – wird rückgängig gemacht …"
            UndoStage.Done -> " – rückgängig gemacht"
            is UndoStage.Partly -> " – teilweise rückgängig gemacht, ${stage.left.size} nicht zurückgesetzt"
            is UndoStage.Failed -> " – Rückgängig fehlgeschlagen: ${stage.message}"
            else -> ""
        }
        return "$head $body ${modeLabel(job.query)}$undoText"
    }
}

/**
 * The replacement flow of the page. Runs in the view model's [scope], so a dry run or run survives
 * "Zurück" like the search; [onChanged] gets the files a finished run changed.
 */
internal class ContentReplaceFlow(
    private val scope: CoroutineScope,
    private val onChanged: (Collection<String>) -> Unit,
) {
    var confirm by mutableStateOf<ReplaceConfirm?>(null)
        private set
    var outcome by mutableStateOf<ReplaceOutcome?>(null)
        private set

    /** Bumped on every applied answer, so the outcome recomposes. */
    var version by mutableIntStateOf(0)
        private set
    var message by mutableStateOf<String?>(null)
        private set
    var taskId by mutableStateOf<String?>(null)
        private set

    val running: Boolean
        get() = taskId != null

    private var previewTask: String? = null
    private var previewJob: Job? = null

    private fun busy(): Boolean {
        if (confirm != null || running) {
            message = "Ein Ersetzen läuft bereits; bitte erst beenden oder abbrechen."
            return true
        }
        message = null
        return false
    }

    /** "In Dateien ersetzen" of the form: the dry run starts at once. */
    fun request(job: ReplaceJob) {
        if (busy()) return
        preview(job)
    }

    /** "Ersetzen…" of the result: the confirmation asks "Ersetzen durch" first. */
    fun ask(job: ReplaceJob) {
        if (busy()) return
        confirm = ReplaceConfirm(job, ReplaceStage.Asking(job.replacement))
    }

    /** "Weiter" after "Ersetzen durch": the dry run of [text]. */
    fun previewWith(text: String) {
        val current = confirm ?: return
        preview(current.job.copy(replacement = text))
    }

    private fun preview(job: ReplaceJob) {
        confirm = ReplaceConfirm(job, ReplaceStage.Counting(0, 0))
        previewJob = scope.launch {
            val id = try {
                ReplaceApi.start(job.location, job.query, job.replacement, job.hits, dryRun = true)
            } catch (e: CoreException) {
                confirm = ReplaceConfirm(job, ReplaceStage.Failed(e.message ?: e.kind))
                return@launch
            }
            previewTask = id
            val found = ReplaceOutcome(job)
            try {
                pollUntilDone(id) { poll ->
                    found.apply(poll)
                    if (confirm?.job != job) return@pollUntilDone
                    confirm = ReplaceConfirm(
                        job,
                        when {
                            !poll.done -> ReplaceStage.Counting(found.changed.size.toLong(), found.replacements)
                            poll.error != null -> ReplaceStage.Failed(poll.error)
                            poll.canceled -> ReplaceStage.Failed("Die Vorschau wurde abgebrochen.")
                            else -> ReplaceStage.Ready(
                                poll.summary?.filesChanged ?: found.changed.size.toLong(),
                                found.replacements,
                                found.skipped,
                            )
                        },
                    )
                }
            } catch (e: CoreException) {
                confirm = ReplaceConfirm(job, ReplaceStage.Failed(e.message ?: e.kind))
            } finally {
                if (previewTask == id) previewTask = null
                clearTask(id)
            }
        }
    }

    /** "Ersetzen" in the confirmation: the run and its outcome. */
    fun apply() {
        val current = confirm ?: return
        if (current.stage !is ReplaceStage.Ready) return
        confirm = null
        val job = current.job
        scope.launch {
            val id = try {
                ReplaceApi.start(job.location, job.query, job.replacement, job.hits, dryRun = false)
            } catch (e: CoreException) {
                message = e.message ?: e.kind
                return@launch
            }
            val fresh = ReplaceOutcome(job)
            outcome = fresh
            taskId = id
            try {
                pollUntilDone(id) { poll ->
                    fresh.apply(poll)
                    version++
                }
                onChanged(fresh.changed.map { it.first })
            } catch (e: CoreException) {
                message = e.message ?: e.kind
            } finally {
                if (taskId == id) taskId = null
                clearTask(id)
            }
        }
    }

    /** "Abbrechen" or closing the confirmation: its dry run stops. */
    fun dismissConfirm() {
        confirm = null
        previewJob?.cancel()
        previewTask?.let { id -> detached.launch { cancelQuietly(id) } }
        previewTask = null
    }

    /** [Abbrechen] of a run: it ends after the current file; written files stay restorable. */
    fun cancel() {
        val id = taskId ?: return
        detached.launch { cancelQuietly(id) }
    }

    /** "Rückgängig" of the last run. */
    fun undo() {
        val current = outcome ?: return
        val stage = current.undo as? UndoStage.Available ?: return
        current.undo = UndoStage.Running
        version++
        scope.launch {
            current.undo = try {
                val result = ReplaceApi.undo(stage.undoId)
                if (result.skipped.isEmpty()) {
                    UndoStage.Done
                } else {
                    UndoStage.Partly(result.skipped.map { SkippedFile(it.rel, "", "nicht zurückgesetzt: ${it.reason}") })
                }
            } catch (e: CoreException) {
                UndoStage.Failed(e.message ?: e.kind)
            }
            version++
        }
    }

    /** The view model is cleared: a dry run and a run stop. */
    fun closed() {
        dismissConfirm()
        cancel()
    }

    private suspend fun pollUntilDone(id: String, onPoll: (ReplacePoll) -> Unit) {
        var since = 0L
        while (true) {
            val answer = ReplaceApi.poll(id, since)
            onPoll(answer)
            since = answer.next
            if (answer.done) return
            if (answer.events.size < POLL_FULL) delay(POLL_INTERVAL_MS)
        }
    }

    private suspend fun clearTask(id: String) {
        withContext(NonCancellable) {
            try {
                // The outcome lives here; the finished task need not stay in the task list.
                FilesApi.clearFinishedTasks(listOf(id))
            } catch (e: CoreException) {
                // Already removed.
            }
        }
    }

    private suspend fun cancelQuietly(id: String) {
        try {
            ReplaceApi.cancel(id)
        } catch (e: CoreException) {
            // Already finished or gone.
        }
    }

    private companion object {
        val detached = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        const val POLL_INTERVAL_MS = 250L
        const val POLL_FULL = 2000
    }
}
