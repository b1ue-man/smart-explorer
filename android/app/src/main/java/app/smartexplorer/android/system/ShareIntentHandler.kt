package app.smartexplorer.android.system

import android.content.ContentResolver
import android.content.Intent
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.core.content.IntentCompat
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.api.ImportFile
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.CoreState
import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.MainTab
import app.smartexplorer.android.ui.NavRequest
import app.smartexplorer.android.ui.common.Snackbars
import java.io.FileNotFoundException
import java.io.IOException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Receives files shared from other apps (SEND/SEND_MULTIPLE, spec F9 "Empfangen"). The
 * descriptors are opened right away (the URI grant lives with the receiving activity); the Files
 * screen then asks for a target place and hands the detached descriptors to `fs.import`.
 */
object ShareIntentHandler {
    private const val TAG = "SmartExplorer"

    /** One shared content, already opened for reading. */
    class SharedFile(val name: String, val size: Long?, internal val descriptor: ParcelFileDescriptor)

    /** State of the latest received share. */
    sealed interface Incoming {
        /** Descriptors are being opened (identity marks one share). */
        class Opening(val count: Int) : Incoming

        /** Ready for a target; [unreadable] contents could not be opened. */
        class Ready(val files: List<SharedFile>, val unreadable: Int) : Incoming
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val state = MutableStateFlow<Incoming?>(null)
    private var openJob: Job? = null

    /** The received share waiting for a target place, or `null`. */
    val pending: StateFlow<Incoming?> = state.asStateFlow()

    /**
     * Handles SEND/SEND_MULTIPLE; returns `false` for every other intent. Switches to the Files tab,
     * which shows the target picker.
     */
    fun handle(activity: ComponentActivity, intent: Intent): Boolean {
        if (intent.action != Intent.ACTION_SEND && intent.action != Intent.ACTION_SEND_MULTIPLE) return false
        val uris = streams(intent)
        if (uris.isEmpty()) {
            Snackbars.show("Nur geteilte Dateien können gespeichert werden.")
            return true
        }
        discard()
        val app = activity.applicationContext
        val resolver = app.contentResolver
        val ownPackage = app.packageName
        val opening = Incoming.Opening(uris.size)
        state.value = opening
        openJob = scope.launch {
            val opened = uris.mapNotNull { open(resolver, ownPackage, it) }
            // A newer share (or discard) replaced this one meanwhile: release what was opened here.
            if (!state.compareAndSet(opening, Incoming.Ready(opened, uris.size - opened.size))) {
                opened.forEach { closeQuietly(it) }
            }
        }
        AppNav.send(NavRequest.SelectTab(MainTab.Files))
        return true
    }

    /**
     * Starts `fs.import` of the pending files into [targetDir] and returns the task id. The core
     * gets detached duplicates of the descriptors and owns them (api.md §4.3). While the call runs
     * the share is no longer pending; when the core is not ready or rejects the call, it stays (or
     * comes back) pending for another attempt.
     */
    suspend fun importInto(targetDir: String): String {
        val ready = state.value as? Incoming.Ready ?: throw CoreException("invalid", "Keine geteilten Dateien vorhanden")
        if (ready.files.isEmpty()) {
            discard()
            throw CoreException("not_found", "Die geteilten Dateien konnten nicht gelesen werden")
        }
        if (Core.ready.value != CoreState.Ready) throw CoreException("not_initialized", "Der Kern ist noch nicht bereit")
        // Taken atomically: a second tap, a discard or a newer share finds nothing of this one.
        if (!state.compareAndSet(ready, null)) throw CoreException("invalid", "Keine geteilten Dateien vorhanden")
        // Not cancellable: once descriptors are detached, the call that hands them to the core must run.
        return withContext(NonCancellable) {
            val taskId = try {
                FilesApi.importFiles(detachedCopies(ready.files), targetDir)
            } catch (e: CoreException) {
                restore(ready)
                throw e
            }
            ready.files.forEach { closeQuietly(it) }
            taskId
        }
    }

    /** Drops the pending share and closes its descriptors. */
    fun discard() {
        openJob?.cancel()
        openJob = null
        val previous = state.value
        state.value = null
        if (previous is Incoming.Ready) previous.files.forEach { closeQuietly(it) }
    }

    private fun streams(intent: Intent): List<Uri> {
        val uris = LinkedHashSet<Uri>()
        if (intent.action == Intent.ACTION_SEND) {
            IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)?.let { uris += it }
        } else {
            IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)?.let { uris += it }
        }
        // Some apps put the contents only into the ClipData.
        intent.clipData?.let { clip -> for (i in 0 until clip.itemCount) clip.getItemAt(i).uri?.let { uris += it } }
        return uris.filter { it.scheme == ContentResolver.SCHEME_CONTENT }
    }

    /** Duplicates of the shared descriptors, detached for the core; the originals stay open. */
    private fun detachedCopies(files: List<SharedFile>): List<ImportFile> {
        val copies = ArrayList<ParcelFileDescriptor>(files.size)
        try {
            files.forEach { copies += it.descriptor.dup() }
        } catch (e: IOException) {
            copies.forEach { closeQuietly(it) }
            throw CoreException("internal", "Die geteilten Dateien konnten nicht übergeben werden: ${e.message}")
        }
        return files.zip(copies) { file, copy -> ImportFile(fd = copy.detachFd(), name = file.name, size = file.size) }
    }

    /** Puts a share back after a failed import; closes it when a newer share arrived meanwhile. */
    private fun restore(ready: Incoming.Ready) {
        if (!state.compareAndSet(null, ready)) ready.files.forEach { closeQuietly(it) }
    }

    private fun open(resolver: ContentResolver, ownPackage: String, uri: Uri): SharedFile? {
        // This app's own providers would serve the URI with this app's rights (all-files access),
        // not the sender's: a file the sender cannot read must not leave through a share.
        if (isOwn(uri, ownPackage)) {
            Log.w(TAG, "shared content of this app refused: $uri")
            return null
        }
        val descriptor = try {
            resolver.openFileDescriptor(uri, "r")
        } catch (e: FileNotFoundException) {
            Log.w(TAG, "shared content not readable: $uri", e)
            null
        } catch (e: SecurityException) {
            Log.w(TAG, "shared content not permitted: $uri", e)
            null
        } catch (e: RuntimeException) {
            // Any app may share: errors of its provider cross Binder unchanged (IllegalArgument-,
            // IllegalState-, UnsupportedOperationException, ...) and must not end this process.
            Log.w(TAG, "shared content failed: $uri", e)
            null
        } ?: return null
        val (name, size) = describe(resolver, uri)
        val statSize = descriptor.statSize.takeIf { it >= 0 }
        return SharedFile(name ?: uri.lastPathSegment?.substringAfterLast('/') ?: "Geteilte Datei", size ?: statSize, descriptor)
    }

    /** Display name and size via OpenableColumns (android-apis.md §6.5); both may be unknown. */
    private fun describe(resolver: ContentResolver, uri: Uri): Pair<String?, Long?> = try {
        resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { cursor ->
            if (!cursor.moveToFirst()) return@use null to null
            val nameIndex = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
            val sizeIndex = cursor.getColumnIndex(OpenableColumns.SIZE)
            val name = if (nameIndex >= 0 && !cursor.isNull(nameIndex)) cursor.getString(nameIndex) else null
            val size = if (sizeIndex >= 0 && !cursor.isNull(sizeIndex)) cursor.getLong(sizeIndex) else null
            name?.substringAfterLast('/')?.takeIf { it.isNotBlank() } to size
        } ?: (null to null)
    } catch (e: RuntimeException) {
        // Provider without these columns (IllegalArgumentException), refused (SecurityException) or
        // failing otherwise (any exception of the sending app's provider, SQLiteException).
        null to null
    }

    /** URIs of this app's providers (`<package>.*` authorities, with or without a user prefix). */
    private fun isOwn(uri: Uri, ownPackage: String): Boolean {
        val authority = uri.authority?.substringAfterLast('@') ?: return false
        return authority.equals(ownPackage, ignoreCase = true) || authority.startsWith("$ownPackage.", ignoreCase = true)
    }

    private fun closeQuietly(file: SharedFile) {
        closeQuietly(file.descriptor)
    }

    private fun closeQuietly(descriptor: ParcelFileDescriptor) {
        try {
            descriptor.close()
        } catch (e: IOException) {
            Log.w(TAG, "closing a shared descriptor failed", e)
        }
    }
}
