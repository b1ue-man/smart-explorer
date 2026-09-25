package app.smartexplorer.android.ui.files

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.system.ShareIntentHandler
import app.smartexplorer.android.ui.common.ConfirmDialog
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.picker.LocationPickerDialog
import app.smartexplorer.android.ui.sync.MirrorDialog
import app.smartexplorer.android.ui.transfers.TransfersSheet

/** Dialogs, sheets, the target picker, the download progress and the receive picker of "Dateien". */
@Composable
internal fun FilesOverlays(vm: FilesViewModel, tasks: List<TaskInfo>) {
    Dialogs(vm)
    Sheets(vm, tasks)
    vm.picker?.let { request ->
        LocationPickerDialog(
            title = request.title,
            initialLocation = request.initial,
            confirmLabel = request.confirmLabel,
            onPick = { vm.onPicked(request.action, it) },
            onDismiss = { vm.picker = null },
        )
    }
    vm.openProgress?.let { progress ->
        key(progress.taskId) {
            OpenProgressDialog(progress.name, tasks.firstOrNull { it.id == progress.taskId }, onCancel = { vm.cancelOpen() })
        }
    }
    ReceivePicker(vm)
}

@Composable
private fun Dialogs(vm: FilesViewModel) {
    val close = { vm.dialog = null }
    when (val dialog = vm.dialog) {
        is FilesDialog.Name -> NameDialog(
            kind = dialog.kind,
            parent = dialog.parent,
            initialName = dialog.initial,
            isDir = dialog.entry?.isDir ?: (dialog.kind == NameDialogKind.NewFolder),
            onDismiss = close,
            onConfirm = { name ->
                close()
                vm.confirmName(dialog, name)
            },
        )
        is FilesDialog.Delete -> DeleteDialog(
            count = dialog.entries.size,
            canTrash = dialog.canTrash,
            onConfirm = { permanent ->
                close()
                vm.delete(dialog.tabId, dialog.entries.map { it.location }, permanent)
            },
            onDismiss = close,
        )
        is FilesDialog.DeletePermanently -> ConfirmDialog(
            title = "Endgültig löschen?",
            message = "Dieser Ort hat keinen Papierkorb. ${elements(dialog.locations.size)} werden endgültig gelöscht.",
            confirmLabel = "Endgültig löschen",
            destructive = true,
            onConfirm = {
                close()
                vm.delete(dialog.tabId, dialog.locations, permanent = true)
            },
            onDismiss = close,
        )
        is FilesDialog.Conflict -> ConflictDialog(
            names = dialog.names,
            choosable = dialog.choosable,
            onChoice = { conflict ->
                close()
                vm.runTransfer(dialog.request, conflict)
            },
            onDismiss = close,
        )
        is FilesDialog.Extract -> ExtractDialog(
            name = dialog.entry.name,
            onHere = {
                close()
                vm.extract(dialog.tabId, dialog.entry.location, targetDir = null)
            },
            onElsewhere = {
                close()
                vm.tab(dialog.tabId)?.let { vm.requestExtractTo(it, dialog.entry) }
            },
            onDismiss = close,
        )
        is FilesDialog.EditConflict -> EditConflictDialog(
            name = dialog.edit.name,
            canOverwrite = dialog.canOverwrite,
            onOverwrite = {
                close()
                vm.uploadEdit(dialog.edit, mode = "overwrite", force = true)
            },
            onCopy = {
                close()
                vm.uploadEdit(dialog.edit, mode = "copy")
            },
            onDismiss = close,
        )
        is FilesDialog.Text -> TextDialog(
            title = dialog.title,
            text = dialog.text,
            onCopy = {
                close()
                vm.emit(FilesEffect.CopyText("Text", dialog.text))
            },
            onDismiss = close,
        )
        null -> Unit
    }
}

@Composable
private fun Sheets(vm: FilesViewModel, tasks: List<TaskInfo>) {
    val close = { vm.sheet = null }
    when (val sheet = vm.sheet) {
        FilesSheet.Transfers -> TransfersSheet(onDismiss = close)
        FilesSheet.Tabs -> TabsSheet(vm, onDismiss = close)
        FilesSheet.ViewOptions -> ViewOptionsSheet(vm.sortKey, vm.sortDesc, onSort = { key, desc -> vm.setSort(key, desc) }, onDismiss = close)
        is FilesSheet.Filter -> {
            // A closed tab leaves nothing to edit; the sheet then simply does not show.
            val tab = vm.tab(sheet.tabId)
            if (tab != null) FilterSheet(tab, onDismiss = close)
        }
        is FilesSheet.Properties -> PropertiesSheet(
            state = sheet.state,
            tasks = tasks,
            onCopyPath = { vm.copyPaths(sheet.state.locations) },
            onDismiss = { vm.closeProperties() },
        )
        is FilesSheet.Mirror -> MirrorDialog(source = sheet.source, onDismiss = close)
        null -> Unit
    }
}

/** Files shared from another app: pick a target, then import with progress (spec F9). */
@Composable
private fun ReceivePicker(vm: FilesViewModel) {
    val incoming by ShareIntentHandler.pending.collectAsStateWithLifecycle()
    val ready = incoming as? ShareIntentHandler.Incoming.Ready ?: return
    LaunchedEffect(ready) {
        if (ready.files.isEmpty()) {
            Snackbars.show("Die geteilten Inhalte konnten nicht gelesen werden.")
            ShareIntentHandler.discard()
        } else if (ready.unreadable > 0) {
            Snackbars.show("${ready.unreadable} geteilte Inhalte nicht lesbar – sie werden ausgelassen.")
        }
    }
    if (ready.files.isEmpty()) return
    val count = ready.files.size
    LocationPickerDialog(
        title = if (count == 1) "„${ready.files.first().name}“ speichern in…" else "$count Dateien speichern in…",
        initialLocation = vm.activeTab?.writableLocation,
        confirmLabel = "Hier speichern",
        onPick = { target -> vm.importShared(target, count) },
        onDismiss = { ShareIntentHandler.discard() },
    )
}
