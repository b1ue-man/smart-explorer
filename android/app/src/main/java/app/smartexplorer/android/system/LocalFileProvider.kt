package app.smartexplorer.android.system

import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.content.pm.ProviderInfo
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import java.io.File
import java.io.FileNotFoundException
import java.io.IOException

/**
 * Serves local files on the reported storage volumes to other apps (open with, share) without a
 * copy, so edits of the other app land in the original (spec F9, android-apis.md §12).
 *
 * URIs: `content://<package>.localfiles/<path segments of the absolute path>`. Only canonical
 * paths below a mounted volume are served; symlinks are resolved before the check. The provider is
 * not exported: callers reach it only through URI grants from this app, and the framework checks
 * the read/write grant against the open mode before [openFile] runs.
 */
class LocalFileProvider : ContentProvider() {
    override fun onCreate(): Boolean = true

    override fun attachInfo(context: Context, info: ProviderInfo) {
        super.attachInfo(context, info)
        // Same safety rules as androidx FileProvider: grants only, never a public provider.
        if (info.exported) throw SecurityException("LocalFileProvider must not be exported")
        if (!info.grantUriPermissions) throw SecurityException("LocalFileProvider must grant URI permissions")
    }

    override fun getType(uri: Uri): String? = resolve(uri)?.let { mimeTypeOf(it.name) }

    override fun query(
        uri: Uri,
        projection: Array<out String>?,
        selection: String?,
        selectionArgs: Array<out String>?,
        sortOrder: String?,
    ): Cursor? {
        val file = resolve(uri) ?: return null
        val requested = projection ?: DEFAULT_COLUMNS
        val columns = ArrayList<String>(requested.size)
        val values = ArrayList<Any?>(requested.size)
        for (column in requested) {
            when (column) {
                OpenableColumns.DISPLAY_NAME -> {
                    columns += column
                    values += file.name
                }
                OpenableColumns.SIZE -> {
                    columns += column
                    values += file.length()
                }
            }
        }
        return MatrixCursor(columns.toTypedArray(), 1).apply { addRow(values.toTypedArray()) }
    }

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor? {
        val file = resolve(uri) ?: throw FileNotFoundException("Kein freigegebener lokaler Pfad: $uri")
        val flags = try {
            // "w" truncates here (like androidx FileProvider): an editor writing a shorter file
            // must never leave the tail of the old content in the original.
            ParcelFileDescriptor.parseMode(if (mode == "w") "wt" else mode)
        } catch (e: IllegalArgumentException) {
            throw FileNotFoundException("Ungültiger Zugriffsmodus: $mode")
        }
        return ParcelFileDescriptor.open(file, flags)
    }

    // Other apps may read and write file contents only; the provider never creates or deletes.
    override fun insert(uri: Uri, values: ContentValues?): Uri? = null

    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int = 0

    override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<out String>?): Int = 0

    /** The canonical file behind [uri], or `null` when it is not below a mounted volume. */
    private fun resolve(uri: Uri): File? {
        val ctx = context ?: return null
        if (uri.authority != authority(ctx)) return null
        val segments = uri.pathSegments
        if (segments.isEmpty()) return null
        return allowedFile(ctx, File("/" + segments.joinToString("/")))
    }

    companion object {
        private val DEFAULT_COLUMNS = arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)
        private const val FALLBACK_MIME = "application/octet-stream"

        fun authority(context: Context): String = context.packageName + ".localfiles"

        /** Content URI for a local [path], or `null` when the path is not below a mounted volume. */
        fun uriFor(context: Context, path: String): Uri? {
            val file = allowedFile(context, File(path)) ?: return null
            val builder = Uri.Builder().scheme("content").authority(authority(context))
            file.path.split('/').filter { it.isNotEmpty() }.forEach { builder.appendPath(it) }
            return builder.build()
        }

        /** MIME type from the file extension (MimeTypeMap), `application/octet-stream` if unknown. */
        fun mimeTypeOf(name: String): String {
            val ext = name.substringAfterLast('.', "").lowercase()
            if (ext.isEmpty()) return FALLBACK_MIME
            return MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext) ?: FALLBACK_MIME
        }

        private fun allowedFile(context: Context, file: File): File? {
            val canonical = try {
                file.canonicalFile
            } catch (e: IOException) {
                return null
            }
            val inside = Storage.volumes(context).any { volume ->
                val root = try {
                    File(volume.path).canonicalFile
                } catch (e: IOException) {
                    return@any false
                }
                canonical.path.startsWith(root.path.trimEnd('/') + "/")
            }
            return if (inside) canonical else null
        }
    }
}
