package app.smartexplorer.android.ui.files

import androidx.lifecycle.viewModelScope
import app.smartexplorer.android.api.EditInfo
import app.smartexplorer.android.api.FetchResult
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.api.MaterializeResult
import app.smartexplorer.android.api.UploadConflict
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.Entry
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.system.ShareIntentHandler
import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.NavRequest
import app.smartexplorer.android.ui.common.Snackbars
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

// File operations of the Files tab (spec F7, F9, F10). Every operation reports its end as a
// snackbar, a progress display or a visible list change (spec C "Rückmeldung").

/** Runs [block]; a core error becomes a snackbar "[failure]: message". */
private fun FilesViewModel.launchAction(failure: String, block: suspend CoroutineScope.() -> Unit): Job =
    viewModelScope.launch {
        try {
            block()
        } catch (e: CoreException) {
            Snackbars.show("$failure: ${e.message ?: e.kind}")
        }
    }

/** Snackbar for a finished task; [Details] opens the transfers sheet. */
private fun reportTask(task: TaskInfo, success: String, failure: String) {
    val details = { AppNav.send(NavRequest.ShowTransfers) }
    when {
        task.state == "done" && task.errors.isEmpty() -> Snackbars.show(success)
        task.state == "done" -> Snackbars.show("$success – ${task.errors.size} Fehler", "Details", details)
        task.state == "canceled" -> Snackbars.show("$failure: abgebrochen")
        else -> Snackbars.show("$failure: ${task.message ?: "unbekannter Fehler"}", "Details", details)
    }
}

// ---- open, share ----

/** Tap on a row: folders and ZIP archives open inside, files open in another app. */
internal fun FilesViewModel.openEntry(tab: BrowserTab, entry: Entry, local: Boolean) {
    when {
        entry.isDir -> tab.open(entry.location)
        // ZIP archives browse like folders (read-only); the core lists them as `zip://` places.
        entry.ext.removePrefix(".").equals("zip", ignoreCase = true) -> tab.open(entry.location)
        else -> openFile(entry, local, chooser = false)
    }
}

/**
 * Local files open in place through the LocalFileProvider; remote files are downloaded first
 * ("Wird geladen…", cancellable) and registered for upload after changes.
 */
internal fun FilesViewModel.openFile(entry: Entry, local: Boolean, chooser: Boolean) = launchAction("Öffnen fehlgeschlagen") {
    if (local) {
        try {
            val target = FilesApi.open(entry.location)
            emit(FilesEffect.Open(target.localPath, target.mime, chooser))
            return@launchAction
        } catch (e: CoreException) {
            // Not a plain local file (e.g. inside a ZIP): download it like a remote file.
            if (e.kind != "unsupported") throw e
        }
    }
    val taskId = FilesApi.fetch(entry.location)
    openProgress = OpenProgress(taskId, entry.name)
    val task = try {
        FilesApi.awaitTask(taskId)
    } finally {
        if (openProgress?.taskId == taskId) openProgress = null
    }
    when (task.state) {
        "done" -> {
            val result = FilesApi.resultOf(task, FetchResult.serializer())
                ?: throw CoreException("internal", "Keine lokale Kopie erhalten")
            emit(FilesEffect.Open(result.localPath, result.mime, chooser))
            reloadEdits()
        }
        "canceled" -> Unit
        else -> reportTask(task, "", "Laden fehlgeschlagen")
    }
}

/** Offers the selected files to other apps (remote files are loaded into the cache first). */
internal fun FilesViewModel.share(entries: List<Entry>) = launchAction("Teilen fehlgeschlagen") {
    val files = entries.filterNot { it.isDir }
    if (files.isEmpty()) {
        Snackbars.show("Ordner lassen sich nicht teilen – bitte Dateien auswählen.")
        return@launchAction
    }
    val task = FilesApi.awaitTask(FilesApi.materialize(files.map { it.location }))
    if (task.state != "done") {
        reportTask(task, "", "Teilen fehlgeschlagen")
        return@launchAction
    }
    val paths = FilesApi.resultOf(task, MaterializeResult.serializer())?.paths.orEmpty()
    if (paths.isEmpty()) throw CoreException("internal", "Keine Dateien zum Teilen erhalten")
    emit(FilesEffect.Share(paths, skippedFolders = entries.size - files.size))
}

/** [Abbrechen] of "Wird geladen…". */
internal fun FilesViewModel.cancelOpen() {
    val taskId = openProgress?.taskId ?: return
    openProgress = null
    viewModelScope.launch { cancelQuietly(taskId) }
}

internal fun FilesViewModel.copyPaths(locations: List<String>) {
    emit(FilesEffect.CopyText("Pfad", locations.joinToString("\n")))
}

// ---- clipboard, copy, move ----

internal fun FilesViewModel.copyToClip(tab: BrowserTab, entries: List<Entry>, move: Boolean) {
    val window = tab.scan
    clip = Clip(
        sources = entries.map { it.location },
        move = move,
        local = window?.local ?: (tab.listing?.isLocal == true),
        filter = window?.transferFilter,
        baseDir = if (window != null && window.filter != null) window.root else null,
    )
    tab.selection.clear()
}

/** "Hier einfügen" into [tab]; cut items move only between local places (desktop rule). */
internal fun FilesViewModel.paste(tab: BrowserTab) {
    val current = clip ?: return
    val target = tab.writableLocation
    if (target == null) {
        Snackbars.show("Hier kann nichts eingefügt werden (nur lesen).")
        return
    }
    if (current.move && (!current.local || tab.listing?.isLocal != true)) {
        Snackbars.show("Verschieben zu Remote wird nicht unterstützt – bitte kopieren")
        return
    }
    startTransfer(TransferRequest(current.sources, target, current.move, current.filter, current.baseDir, clearsClip = current.move))
}

/** "Kopieren nach…" / "Verschieben nach…": target picker, suggesting the other pane. */
internal fun FilesViewModel.requestTransferTo(tab: BrowserTab, entries: List<Entry>, move: Boolean) {
    val window = tab.scan
    picker = PickerRequest(
        title = if (move) "Verschieben nach…" else "Kopieren nach…",
        confirmLabel = if (move) "Hierhin verschieben" else "Hierhin kopieren",
        initial = otherPaneLocation(tab) ?: tab.listing?.location,
        action = PickerAction.Transfer(
            sources = entries.map { it.location },
            move = move,
            filter = window?.transferFilter,
            baseDir = if (window != null && window.filter != null) window.root else null,
        ),
    )
    tab.selection.clear()
}

/** Checks name conflicts first: local→local asks skip/replace/keep both, remote targets number names. */
internal fun FilesViewModel.startTransfer(request: TransferRequest) = launchAction(transferFailure(request)) {
    val check = FilesApi.conflicts(request.sources, request.targetDir)
    if (check.names.isEmpty()) {
        transferNow(request, conflict = "keepBoth")
    } else {
        dialog = FilesDialog.Conflict(request, check.names, check.choosable)
    }
}

/** [conflict] `skip|replace|keepBoth` (effective only local→local). */
internal fun FilesViewModel.runTransfer(request: TransferRequest, conflict: String) =
    launchAction(transferFailure(request)) { transferNow(request, conflict) }

private suspend fun FilesViewModel.transferNow(request: TransferRequest, conflict: String) {
    val taskId = FilesApi.transfer(
        sources = request.sources,
        targetDir = request.targetDir,
        mode = if (request.move) "move" else "copy",
        conflict = conflict,
        filter = request.filter,
        baseDir = request.baseDir,
    )
    if (request.clearsClip) clip = null
    val task = FilesApi.awaitTask(taskId)
    val verb = if (request.move) "Verschoben" else "Kopiert"
    reportTask(task, "$verb: ${elements(request.sources.size)}", transferFailure(request))
    refreshAfterChange()
}

private fun transferFailure(request: TransferRequest) =
    if (request.move) "Verschieben fehlgeschlagen" else "Kopieren fehlgeschlagen"

// ---- delete, rename, create ----

internal fun FilesViewModel.requestDelete(tab: BrowserTab, entries: List<Entry>) {
    if (entries.isEmpty()) return
    dialog = FilesDialog.Delete(tab.id, entries, canTrash = tab.listing?.canTrash == true)
}

/** Trash by default; a place without trash answers `unsupported` and asks for permanent deletion. */
internal fun FilesViewModel.delete(tabId: Long, locations: List<String>, permanent: Boolean) = launchAction("Löschen fehlgeschlagen") {
    val taskId = try {
        FilesApi.delete(locations, permanent)
    } catch (e: CoreException) {
        if (e.kind == "unsupported" && !permanent) {
            dialog = FilesDialog.DeletePermanently(tabId, locations)
            return@launchAction
        }
        throw e
    }
    tab(tabId)?.selection?.clear()
    val task = FilesApi.awaitTask(taskId)
    val done = if (permanent) "Endgültig gelöscht" else "In den Papierkorb verschoben"
    reportTask(task, "$done: ${elements(locations.size)}", "Löschen fehlgeschlagen")
    refreshAfterChange()
}

internal fun FilesViewModel.requestRename(tab: BrowserTab, entry: Entry) {
    // The folder of an entry is known for direct children only (Kotlin never builds locations).
    val parent = if (entry.depth == 0) tab.listing?.location else null
    dialog = FilesDialog.Name(NameDialogKind.Rename, tab.id, parent, entry, entry.name)
}

internal fun FilesViewModel.requestCreate(tab: BrowserTab, folder: Boolean) {
    val parent = tab.writableLocation ?: return
    dialog = if (folder) {
        FilesDialog.Name(NameDialogKind.NewFolder, tab.id, parent, null, "Neuer Ordner")
    } else {
        FilesDialog.Name(NameDialogKind.NewFile, tab.id, parent, null, "Neue Datei.txt")
    }
}

internal fun FilesViewModel.confirmName(request: FilesDialog.Name, name: String) = launchAction(
    when (request.kind) {
        NameDialogKind.Rename -> "Umbenennen fehlgeschlagen"
        NameDialogKind.NewFolder -> "Ordner nicht angelegt"
        NameDialogKind.NewFile -> "Datei nicht angelegt"
    },
) {
    when (request.kind) {
        NameDialogKind.Rename -> {
            val entry = request.entry ?: return@launchAction
            val renamed = FilesApi.rename(entry.location, name)
            Snackbars.show("Umbenannt in „${renamed.name}“")
        }
        NameDialogKind.NewFolder -> FilesApi.mkdir(request.parent ?: return@launchAction, name)
        NameDialogKind.NewFile -> FilesApi.newFile(request.parent ?: return@launchAction, name)
    }
    tab(request.tabId)?.selection?.clear()
    refreshAfterChange()
}

// ---- properties, ZIP, favorites ----

/** Properties sheet; folders are measured recursively by a task with progress. */
internal fun FilesViewModel.showProperties(entries: List<Entry>) {
    if (entries.isEmpty()) return
    val locations = entries.map { it.location }
    val title = entries.singleOrNull()?.name ?: elements(entries.size)
    sheet = FilesSheet.Properties(PropertiesState(title, entries, locations, taskId = null))
    viewModelScope.launch {
        var taskId: String? = null
        var error: String? = null
        try {
            taskId = FilesApi.properties(locations)
        } catch (e: CoreException) {
            error = e.message ?: e.kind
        }
        val open = sheet as? FilesSheet.Properties
        if (open != null && open.state.locations == locations && open.state.taskId == null) {
            sheet = FilesSheet.Properties(open.state.copy(taskId = taskId, error = error))
        } else if (taskId != null) {
            // The sheet was closed before the measurement started.
            cancelQuietly(taskId)
        }
    }
}

private suspend fun cancelQuietly(taskId: String) {
    try {
        FilesApi.cancelTask(taskId)
    } catch (e: CoreException) {
        // Already finished: nothing to stop.
    }
}

/** Properties of the listed folder itself (menu "Eigenschaften"). */
internal fun FilesViewModel.showFolderProperties(tab: BrowserTab) {
    val location = tab.listing?.location ?: return
    launchAction("Eigenschaften nicht verfügbar") { showProperties(listOf(FilesApi.stat(location))) }
}

/** Closing the sheet stops a still running measurement. */
internal fun FilesViewModel.closeProperties() {
    val state = (sheet as? FilesSheet.Properties)?.state
    sheet = null
    val taskId = state?.taskId ?: return
    viewModelScope.launch { cancelQuietly(taskId) }
}

internal fun FilesViewModel.requestExtract(tab: BrowserTab, entry: Entry) {
    dialog = FilesDialog.Extract(tab.id, entry)
}

internal fun FilesViewModel.requestExtractTo(tab: BrowserTab, entry: Entry) {
    picker = PickerRequest(
        title = "Entpacken nach…",
        confirmLabel = "Hierhin entpacken",
        initial = otherPaneLocation(tab) ?: tab.listing?.location,
        action = PickerAction.ExtractTo(tab.id, entry.location),
    )
}

/** [targetDir] `null` = folder next to the archive ("Hierher"). */
internal fun FilesViewModel.extract(tabId: Long, location: String, targetDir: String?) = launchAction("Entpacken fehlgeschlagen") {
    val task = FilesApi.awaitTask(FilesApi.extract(location, targetDir))
    tab(tabId)?.selection?.clear()
    reportTask(task, "Entpackt", "Entpacken fehlgeschlagen")
    refreshAfterChange()
}

internal fun FilesViewModel.toggleFavorite(location: String) = launchAction("Favorit nicht geändert") {
    val favorite = FilesApi.toggleFavorite(location)
    Snackbars.show(if (favorite) "Zu Favoriten hinzugefügt" else "Aus Favoriten entfernt")
    reloadRoots()
}

/** "Zu Favoriten" for selected folders; already favored ones stay. */
internal fun FilesViewModel.addFavorites(tab: BrowserTab, entries: List<Entry>) = launchAction("Favoriten nicht geändert") {
    var added = 0
    for (entry in entries.filter { it.isDir }) {
        if (!FilesApi.isFavorite(entry.location) && FilesApi.toggleFavorite(entry.location)) added++
    }
    tab.selection.clear()
    Snackbars.show(if (added == 0) "Bereits in den Favoriten" else "Zu Favoriten hinzugefügt: $added")
    reloadRoots()
}

/** Picker result for [PickerAction]. */
internal fun FilesViewModel.onPicked(action: PickerAction, location: String) {
    picker = null
    when (action) {
        is PickerAction.Transfer -> startTransfer(TransferRequest(action.sources, location, action.move, action.filter, action.baseDir))
        is PickerAction.ExtractTo -> extract(action.tabId, action.location, location)
    }
}

// ---- remote edits, received shares ----

/** Uploads a changed remote copy; a remote change since opening asks how to resolve (spec F9). */
internal fun FilesViewModel.uploadEdit(edit: EditInfo, mode: String, force: Boolean = false) = launchAction("Hochladen fehlgeschlagen") {
    val task = FilesApi.awaitTask(FilesApi.uploadEdit(edit.editId, mode, force))
    val conflict = task.state == "failed" && FilesApi.resultOf(task, UploadConflict.serializer())?.conflict == true
    if (conflict) {
        dialog = FilesDialog.EditConflict(edit)
    } else {
        reportTask(task, "Hochgeladen: ${edit.name}", "Hochladen fehlgeschlagen")
        refreshAfterChange()
    }
    reloadEdits()
}

internal fun FilesViewModel.discardEdit(edit: EditInfo) = launchAction("Verwerfen fehlgeschlagen") {
    FilesApi.discardEdit(edit.editId)
    reloadEdits()
}

/** Saves the files shared from another app into [targetDir] (spec F9 "Empfangen"). */
internal fun FilesViewModel.importShared(targetDir: String, count: Int) = launchAction("Speichern fehlgeschlagen") {
    val task = FilesApi.awaitTask(ShareIntentHandler.importInto(targetDir))
    reportTask(task, if (count == 1) "Gespeichert" else "Gespeichert: $count Dateien", "Speichern fehlgeschlagen")
    refreshAfterChange()
}

/** [Details] of "n Ordner nicht lesbar" in the recursive view. */
internal fun FilesViewModel.showScanIssues(taskId: String) = launchAction("Details nicht verfügbar") {
    dialog = FilesDialog.Text("Nicht lesbare Ordner", FilesApi.scanIssues(taskId))
}

// ---- selection ----

/** "Alle auswählen": all rows of the flat list, or every row of the recursive view. */
internal fun FilesViewModel.selectAll(tab: BrowserTab) {
    val window = tab.scan
    if (window == null) {
        tab.listing?.entries?.forEach { tab.selection[it.location] = it }
        return
    }
    launchAction("Auswahl nicht möglich") {
        val rows = window.fetchAll(MAX_SELECT_ALL)
        if (rows == null) {
            Snackbars.show("Zu viele Treffer für „Alle auswählen“ – Filter enger fassen.")
            return@launchAction
        }
        rows.forEach { tab.selection[it.location] = it }
    }
}

internal fun invertSelection(tab: BrowserTab) {
    val shown = tab.shownEntries()
    val next = shown.filter { it.location !in tab.selection }
    tab.selection.clear()
    next.forEach { tab.selection[it.location] = it }
}

private const val MAX_SELECT_ALL = 20_000
