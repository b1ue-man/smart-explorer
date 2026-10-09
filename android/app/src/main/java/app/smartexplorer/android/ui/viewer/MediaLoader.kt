package app.smartexplorer.android.ui.viewer

import app.smartexplorer.android.api.FetchResult
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.Entry
import java.io.File
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext

/** A medium available as a local file (original or registered remote copy). */
internal data class LocalMedia(val path: String, val mime: String?)

/** Load state of one viewer page. */
internal sealed interface PageLoad {
    /** [taskId]: the running download of a remote file, `null` while resolving a local one. */
    data class Loading(val taskId: String? = null) : PageLoad

    data class Ready(val media: LocalMedia) : PageLoad

    data class Failed(val message: String) : PageLoad
}

/**
 * Makes [entry] available as a local file, like "Öffnen" does (FileActions.openFile): local
 * places open in place (`fs.open`); remote files and entries inside archives are downloaded into
 * the registered copy (`fs.fetch`, changes made by another app are uploaded as before).
 * [onTask] reports the download task. Cancelling the caller cancels the download.
 */
internal suspend fun loadMedia(entry: Entry, local: Boolean, onTask: (String) -> Unit): LocalMedia {
    if (local) {
        try {
            val target = FilesApi.open(entry.location)
            return LocalMedia(target.localPath, target.mime)
        } catch (e: CoreException) {
            // Not a plain local file (e.g. inside a ZIP): download it like a remote file.
            if (e.kind != "unsupported") throw e
        }
    }
    val taskId = FilesApi.fetch(entry.location)
    onTask(taskId)
    var finished = false
    try {
        val task = FilesApi.awaitTask(taskId)
        finished = true
        return when (task.state) {
            "done" -> FilesApi.resultOf(task, FetchResult.serializer())
                ?.let { LocalMedia(it.localPath, it.mime) }
                ?: throw CoreException("internal", "Keine lokale Kopie erhalten")
            "canceled" -> throw CoreException("canceled", "Laden abgebrochen")
            else -> throw CoreException(task.state, task.message ?: "Laden fehlgeschlagen")
        }
    } finally {
        withContext(NonCancellable) {
            try {
                // Leaving the page stops its download; viewed copies do not pile up in "Übertragungen".
                if (!finished) FilesApi.cancelTask(taskId)
                FilesApi.clearFinishedTasks(listOf(taskId))
            } catch (e: CoreException) {
                // Already finished or removed.
            }
        }
    }
}

/** A loaded copy can vanish later (the core drops old unchanged copies); then it is loaded again. */
internal fun PageLoad?.isUsable(): Boolean = this is PageLoad.Ready && File(media.path).isFile
