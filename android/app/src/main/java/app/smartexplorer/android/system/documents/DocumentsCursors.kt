package app.smartexplorer.android.system.documents

import android.database.MatrixCursor
import android.os.Bundle
import android.provider.DocumentsContract.Document
import android.provider.DocumentsContract.Root
import android.webkit.MimeTypeMap
import app.smartexplorer.android.R
import app.smartexplorer.android.api.DocsChildren
import app.smartexplorer.android.api.DocsItem
import app.smartexplorer.android.api.DocsRoot

/** Rows of the provider's cursors (DRV-M8, strand S2 C3). */
object DocumentsCursors {
    val DEFAULT_ROOT_PROJECTION = arrayOf(
        Root.COLUMN_ROOT_ID,
        Root.COLUMN_DOCUMENT_ID,
        Root.COLUMN_TITLE,
        Root.COLUMN_SUMMARY,
        Root.COLUMN_FLAGS,
        Root.COLUMN_ICON,
    )

    val DEFAULT_DOCUMENT_PROJECTION = arrayOf(
        Document.COLUMN_DOCUMENT_ID,
        Document.COLUMN_DISPLAY_NAME,
        Document.COLUMN_MIME_TYPE,
        Document.COLUMN_SIZE,
        Document.COLUMN_LAST_MODIFIED,
        Document.COLUMN_FLAGS,
    )

    private const val SUMMARY = "Smart Explorer"

    /**
     * `FLAG_SUPPORTS_IS_CHILD` always (folder grants), `FLAG_SUPPORTS_CREATE` where the root
     * folder takes new entries, `FLAG_LOCAL_ONLY` only for vaults in this phone's storage.
     */
    fun rootFlags(root: DocsRoot): Int {
        var flags = Root.FLAG_SUPPORTS_IS_CHILD
        if (root.create) flags = flags or Root.FLAG_SUPPORTS_CREATE
        if (root.localOnly) flags = flags or Root.FLAG_LOCAL_ONLY
        return flags
    }

    fun roots(projection: Array<String>?, roots: List<DocsRoot>): MatrixCursor {
        val cursor = MatrixCursor(projection ?: DEFAULT_ROOT_PROJECTION)
        for (root in roots) {
            cursor.newRow()
                .add(Root.COLUMN_ROOT_ID, root.rootId)
                .add(Root.COLUMN_DOCUMENT_ID, root.documentId)
                .add(Root.COLUMN_TITLE, root.title)
                .add(Root.COLUMN_SUMMARY, SUMMARY)
                .add(Root.COLUMN_FLAGS, rootFlags(root))
                .add(Root.COLUMN_ICON, R.mipmap.ic_launcher)
        }
        return cursor
    }

    fun mimeOf(item: DocsItem): String {
        if (item.dir) return Document.MIME_TYPE_DIR
        val extension = item.name.substringAfterLast('.', "").lowercase()
        return MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension) ?: "application/octet-stream"
    }

    /** Write, rename and delete only where the device lets this phone write. */
    fun documentFlags(item: DocsItem): Int {
        if (!item.writable) return 0
        val isRoot = DocumentIds.parent(item.documentId) == null
        var flags = if (item.dir) Document.FLAG_DIR_SUPPORTS_CREATE else Document.FLAG_SUPPORTS_WRITE
        if (!isRoot) flags = flags or Document.FLAG_SUPPORTS_DELETE or Document.FLAG_SUPPORTS_RENAME
        return flags
    }

    fun addDocument(cursor: MatrixCursor, item: DocsItem) {
        cursor.newRow()
            .add(Document.COLUMN_DOCUMENT_ID, item.documentId)
            .add(Document.COLUMN_DISPLAY_NAME, item.name)
            .add(Document.COLUMN_MIME_TYPE, mimeOf(item))
            .add(Document.COLUMN_SIZE, if (item.dir) null else item.size)
            .add(Document.COLUMN_LAST_MODIFIED, item.mtime.takeIf { it > 0 })
            .add(Document.COLUMN_FLAGS, documentFlags(item))
    }

    fun document(projection: Array<String>?, item: DocsItem): MatrixCursor =
        MatrixCursor(projection ?: DEFAULT_DOCUMENT_PROJECTION).also { addDocument(it, item) }

    /**
     * A listing: `EXTRA_LOADING` while the core still lists the folder (a `docs.changed` for it
     * follows), `EXTRA_ERROR` for an unreachable device or a failed listing. DocumentsUI shows
     * both (NR N15).
     */
    fun children(projection: Array<String>?, listing: DocsChildren): MatrixCursor {
        val extras = Bundle()
        if (listing.loading) extras.putBoolean(android.provider.DocumentsContract.EXTRA_LOADING, true)
        listing.error?.let { extras.putString(android.provider.DocumentsContract.EXTRA_ERROR, it) }
        val cursor = MatrixCursor(projection ?: DEFAULT_DOCUMENT_PROJECTION)
        cursor.extras = extras
        listing.children.forEach { addDocument(cursor, it) }
        return cursor
    }
}
