package app.smartexplorer.android.system

import android.content.ContentResolver
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract
import app.smartexplorer.android.ui.NavRequest
import java.io.File

/**
 * "In Smart Explorer öffnen" (spec G6, desktop: Explorer context menu): another app opens a folder
 * – or a file, whose folder is shown – through ACTION_VIEW. Only local folders on the reported
 * volumes are accepted (docs/refs/android-open-with.md).
 */
object OpenWithIntent {
    private const val EXTERNAL_STORAGE_AUTHORITY = "com.android.externalstorage.documents"

    sealed interface Outcome {
        data class Open(val request: NavRequest.OpenLocation) : Outcome
        data object Refused : Outcome
    }

    /** `null` when [intent] is not an open-with request at all. */
    fun outcome(context: Context, intent: Intent): Outcome? {
        if (intent.action != Intent.ACTION_VIEW) return null
        val uri = intent.data ?: return null
        val path = when (uri.scheme) {
            ContentResolver.SCHEME_FILE -> uri.path
            ContentResolver.SCHEME_CONTENT -> documentPath(context, uri)
            else -> return null
        } ?: return Outcome.Refused
        val accepted = OpenWithTarget.acceptedPath(path, Storage.volumes(context)) ?: return Outcome.Refused
        val file = File(accepted)
        val folder = when {
            file.isDirectory -> accepted
            file.isFile -> file.parent ?: return Outcome.Refused
            else -> return Outcome.Refused
        }
        return Outcome.Open(NavRequest.OpenLocation(folder))
    }

    private fun documentPath(context: Context, uri: Uri): String? {
        if (uri.authority != EXTERNAL_STORAGE_AUTHORITY) return null
        val documentId = try {
            if (DocumentsContract.isDocumentUri(context, uri)) {
                DocumentsContract.getDocumentId(uri)
            } else {
                DocumentsContract.getTreeDocumentId(uri)
            }
        } catch (e: IllegalArgumentException) {
            return null
        }
        return OpenWithTarget.pathForDocumentId(documentId, Storage.volumes(context))
    }
}
