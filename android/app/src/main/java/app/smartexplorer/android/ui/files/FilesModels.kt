package app.smartexplorer.android.ui.files

import app.smartexplorer.android.api.EditInfo
import app.smartexplorer.android.core.Entry
import app.smartexplorer.android.core.FilterSpec
import app.smartexplorer.android.core.SortSpec
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Location shown in the active pane of "Dateien" (`null` before the first listing), for other
 * screens that preselect "the current place" (e.g. the storage analysis).
 */
object FilesLocation {
    private val state = MutableStateFlow<String?>(null)
    val current: StateFlow<String?> = state.asStateFlow()

    internal fun set(location: String?) {
        state.value = location
    }
}

/** Pages that replace the file browser inside the Files tab. */
internal enum class FilesPage { Browser, Trash, FolderSearch }

/** List options shared by all tabs. */
internal data class ViewOptions(val showHidden: Boolean, val sort: SortSpec)

/** Place in a tab's history with the list position to restore. */
internal data class HistoryItem(val location: String, val index: Int = 0, val offset: Int = 0)

/**
 * Copied or cut items for "Hier einfügen" (spec F7). [filter] and [baseDir] are set when the
 * selection came from a filtered recursive view: only matching files are copied, relative to
 * [baseDir] (desktop rule).
 */
internal data class Clip(
    val sources: List<String>,
    val move: Boolean,
    val local: Boolean,
    val filter: FilterSpec? = null,
    val baseDir: String? = null,
)

/** One copy or move into [targetDir]. */
internal data class TransferRequest(
    val sources: List<String>,
    val targetDir: String,
    val move: Boolean,
    val filter: FilterSpec? = null,
    val baseDir: String? = null,
    /** Clears the clipboard once the move started ("Ausschneiden" is one-shot). */
    val clearsClip: Boolean = false,
)

/** Kind of the name dialog (rename, new folder, new file). */
internal enum class NameDialogKind { Rename, NewFolder, NewFile }

/** Modal dialogs of the Files tab. */
internal sealed interface FilesDialog {
    /** [siblings]: exact names next to a renamed entry (case-only renames onto them clash). */
    data class Name(
        val kind: NameDialogKind,
        val tabId: Long,
        val parent: String?,
        val entry: Entry?,
        val initial: String,
        val siblings: Set<String> = emptySet(),
    ) : FilesDialog

    /**
     * Deletes [entries]. [folderCount] > 0: folders selected in a filtered recursive view, which
     * stand for their whole content; ticking them deletes [withFolders] instead.
     */
    data class Delete(
        val tabId: Long,
        val entries: List<Entry>,
        val canTrash: Boolean,
        val folderCount: Int = 0,
        val withFolders: List<Entry> = entries,
    ) : FilesDialog

    /** The place has no trash (`unsupported`): ask for permanent deletion. */
    data class DeletePermanently(val tabId: Long, val locations: List<String>) : FilesDialog

    data class Conflict(val request: TransferRequest, val names: List<String>, val choosable: Boolean) : FilesDialog

    data class Extract(val tabId: Long, val entry: Entry) : FilesDialog

    /** The remote file changed since opening. */
    data class EditConflict(val edit: EditInfo) : FilesDialog

    data class Text(val title: String, val text: String) : FilesDialog
}

/** What a picked target place is used for. */
internal sealed interface PickerAction {
    data class Transfer(val sources: List<String>, val move: Boolean, val filter: FilterSpec?, val baseDir: String?) :
        PickerAction

    data class ExtractTo(val tabId: Long, val location: String) : PickerAction
}

internal data class PickerRequest(val title: String, val confirmLabel: String, val initial: String?, val action: PickerAction)

/** Bottom sheets of the Files tab. */
internal sealed interface FilesSheet {
    data object Transfers : FilesSheet

    data object Tabs : FilesSheet

    data object ViewOptions : FilesSheet

    data class Filter(val tabId: Long) : FilesSheet

    data class Properties(val state: PropertiesState) : FilesSheet

    /** „Spiegeln nach…“ for [source] (spec F14, dialog from the Sync block). */
    data class Mirror(val source: String) : FilesSheet
}

/** Properties of one or more entries (or the current folder), computed by a task. */
internal data class PropertiesState(
    val title: String,
    val entries: List<Entry>,
    val locations: List<String>,
    val taskId: String?,
    val error: String? = null,
)

/** Remote file being downloaded for "Öffnen" (spec F9: "Wird geladen…" with [Abbrechen]). */
internal data class OpenProgress(val taskId: String, val name: String)

/** One-shot actions that need the current Activity context. */
internal sealed interface FilesEffect {
    data class Open(val localPath: String, val mime: String?, val chooser: Boolean) : FilesEffect

    data class Share(val paths: List<String>, val skippedFolders: Int) : FilesEffect

    data class CopyText(val label: String, val text: String) : FilesEffect
}

/** "1 Element" / "3 Elemente". */
internal fun elements(count: Int): String = if (count == 1) "1 Element" else "$count Elemente"
