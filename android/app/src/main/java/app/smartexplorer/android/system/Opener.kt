package app.smartexplorer.android.system

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.util.Log
import androidx.core.content.FileProvider
import java.io.File
import java.io.IOException

/**
 * Hands local files to other apps (android-apis.md §6): local originals through
 * [LocalFileProvider], cache copies (`cache/open`, `cache/share`) through the androidx
 * FileProvider `${applicationId}.files`. Always `startActivity` + [ActivityNotFoundException].
 */
object Opener {
    private const val TAG = "SmartExplorer"

    /** Result of an attempt to hand files to another app. */
    enum class Outcome { Started, NoApp, NotShareable }

    /**
     * Opens [localPath] in another app with read and write access (changes land in the original or,
     * for remote files, in the registered cache copy). [chooser] always shows the app picker.
     */
    fun open(context: Context, localPath: String, mime: String?, chooser: Boolean): Outcome {
        val uri = uriFor(context, localPath) ?: return Outcome.NotShareable
        val type = mime?.takeIf { it.isNotBlank() } ?: LocalFileProvider.mimeTypeOf(File(localPath).name)
        val view = Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(uri, type)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
        }
        return start(context, if (chooser) Intent.createChooser(view, "Öffnen mit") else view)
    }

    /** Offers [localPaths] (files only) to the system share sheet, read-only. */
    fun share(context: Context, localPaths: List<String>): Outcome {
        val uris = localPaths.map { uriFor(context, it) ?: return Outcome.NotShareable }
        if (uris.isEmpty()) return Outcome.NotShareable
        val mimeType = commonType(localPaths.map { LocalFileProvider.mimeTypeOf(File(it).name) }.distinct())
        val send = if (uris.size == 1) {
            Intent(Intent.ACTION_SEND).apply {
                type = mimeType
                putExtra(Intent.EXTRA_STREAM, uris.first())
                clipData = ClipData.newRawUri(null, uris.first())
            }
        } else {
            Intent(Intent.ACTION_SEND_MULTIPLE).apply {
                type = mimeType
                putParcelableArrayListExtra(Intent.EXTRA_STREAM, ArrayList(uris))
                clipData = ClipData(null, arrayOf(mimeType), ClipData.Item(uris.first())).apply {
                    uris.drop(1).forEach { addItem(ClipData.Item(it)) }
                }
            }
        }
        send.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        return start(context, Intent.createChooser(send, null))
    }

    /** Copies [text] (e.g. a path) to the system clipboard (android-apis.md §9.2). */
    fun copyText(context: Context, label: String, text: String): Boolean {
        val clipboard = context.getSystemService(ClipboardManager::class.java) ?: return false
        clipboard.setPrimaryClip(ClipData.newPlainText(label, text))
        return true
    }

    /** Content URI for a local file: cache copies via FileProvider, originals via [LocalFileProvider]. */
    private fun uriFor(context: Context, localPath: String): Uri? {
        val file = File(localPath)
        val canonical = try {
            file.canonicalFile
        } catch (e: IOException) {
            return null
        }
        val cacheRoot = try {
            context.cacheDir.canonicalFile
        } catch (e: IOException) {
            return null
        }
        if (canonical.path.startsWith(cacheRoot.path + "/")) {
            return try {
                FileProvider.getUriForFile(context, context.packageName + ".files", canonical)
            } catch (e: IllegalArgumentException) {
                // Outside the declared cache-path folders (open/, share/, update/).
                Log.w(TAG, "cache file not shareable: $localPath", e)
                null
            }
        }
        return LocalFileProvider.uriFor(context, canonical.path)
    }

    private fun commonType(types: List<String>): String {
        if (types.size == 1) return types.first()
        val major = types.map { it.substringBefore('/') }.distinct()
        return if (major.size == 1) major.first() + "/*" else "*/*"
    }

    private fun start(context: Context, intent: Intent): Outcome {
        if (context !is Activity) intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        return try {
            context.startActivity(intent)
            Outcome.Started
        } catch (e: ActivityNotFoundException) {
            Outcome.NoApp
        }
    }
}
