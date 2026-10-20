package app.smartexplorer.android.system.documents

import android.app.Application
import android.content.ComponentName
import android.content.Context
import android.content.pm.PackageManager
import android.database.Cursor
import android.os.CancellationSignal
import android.os.Handler
import android.os.HandlerThread
import android.os.ParcelFileDescriptor
import android.os.storage.StorageManager
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import android.provider.DocumentsProvider
import android.util.Log
import androidx.annotation.VisibleForTesting
import app.smartexplorer.android.api.DocsApi
import app.smartexplorer.android.api.DocsBackend
import app.smartexplorer.android.api.NativeDocsBackend
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.CoreState
import app.smartexplorer.android.core.DocsChanged
import app.smartexplorer.android.prefs.AppPrefs
import java.io.FileNotFoundException
import java.io.IOException
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.filterIsInstance
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/**
 * „Ordner für Apps bereitstellen“ (plan 2026-10-17, DRV-M8, strand S2 C3): the system picker and
 * „Dateien“ show the paired devices, released saved connections (SMB too), Google Drive and every
 * released vault while it is unlocked. Other apps open, edit, create, rename and delete there and
 * may keep a folder grant.
 *
 * - Only the system holds `MANAGE_DOCUMENTS`; other apps get single URIs the person picked.
 * - The component is off until the person switches it on ([setEnabled]); the core then answers
 *   only roots the person released, also for a guessed document ID.
 * - `ContentProvider.onCreate` runs before `Application.onCreate` (NR N10): the provider starts
 *   the settings and the core itself. `queryRoots` never uses the network (DocumentsUI waits 5 s).
 * - Descriptors are proxies (`openProxyFileDescriptor`) with a `HandlerThread` per file; see
 *   [ProxyFileCallback]. MediaStore does not index provider content (assumption A11).
 * - Logs name methods and error kinds only, never a document ID or content.
 */
class SeDocumentsProvider : DocumentsProvider() {
    private lateinit var backend: DocsBackend
    private lateinit var authority: String
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    override fun onCreate(): Boolean {
        val app = context?.applicationContext as? Application ?: return false
        AppPrefs.init(app)
        startCore(app)
        authority = DocumentIds.authority(app)
        backend = backendFactory()
        if (watching.compareAndSet(false, true)) watch(app)
        return true
    }

    /** `docs.changed` → content notifications; a pending upload from an earlier process → worker. */
    private fun watch(app: Application) {
        scope.launch {
            Core.events.filterIsInstance<DocsChanged>().collect { onDocsChanged(app, it) }
        }
        scope.launch {
            if (Core.ready.first { it !is CoreState.Starting } !is CoreState.Ready) return@launch
            val pending = try {
                DocsApi.pending().pending
            } catch (e: CoreException) {
                Log.w(TAG, "docs.pending: ${e.kind}")
                0
            }
            if (pending > 0) DocsCommitWorker.enqueue(app)
        }
    }

    @VisibleForTesting
    internal fun onDocsChanged(context: Context, event: DocsChanged) {
        val resolver = context.contentResolver
        if (event.roots) resolver.notifyChange(DocumentsContract.buildRootsUri(authority), null)
        for (id in event.documents) {
            resolver.notifyChange(DocumentsContract.buildChildDocumentsUri(authority, id), null)
            resolver.notifyChange(DocumentsContract.buildDocumentUri(authority, id), null)
        }
        if (event.commitFailed) {
            DocsCommitWorker.failed(context)
            DocsCommitWorker.enqueue(context)
        }
    }

    override fun queryRoots(projection: Array<String>?): Cursor {
        val roots = try {
            backend.roots()
        } catch (e: Exception) {
            // Not ready in time or failed: no roots now; `docs.changed` brings them later.
            Log.w(TAG, "queryRoots: ${e.javaClass.simpleName}")
            emptyList()
        }
        return DocumentsCursors.roots(projection, roots).also {
            it.setNotificationUri(context!!.contentResolver, DocumentsContract.buildRootsUri(authority))
        }
    }

    override fun queryDocument(documentId: String, projection: Array<String>?): Cursor {
        val item = core("queryDocument") { backend.document(checked(documentId)) }
        return DocumentsCursors.document(projection, item)
    }

    override fun queryChildDocuments(parentDocumentId: String, projection: Array<String>?, sortOrder: String?): Cursor {
        val listing = core("queryChildDocuments") { backend.children(checked(parentDocumentId)) }
        return DocumentsCursors.children(projection, listing).also {
            it.setNotificationUri(
                context!!.contentResolver,
                DocumentsContract.buildChildDocumentsUri(authority, parentDocumentId),
            )
        }
    }

    /** By the IDs alone, without I/O. */
    override fun isChildDocument(parentDocumentId: String, documentId: String): Boolean =
        DocumentIds.isChild(parentDocumentId, documentId)

    override fun getDocumentType(documentId: String): String =
        DocumentsCursors.mimeOf(core("getDocumentType") { backend.document(checked(documentId)) })

    override fun openDocument(documentId: String, mode: String, signal: CancellationSignal?): ParcelFileDescriptor {
        val parsed = DocumentIds.openMode(mode)
            ?: throw IllegalArgumentException("Öffnungsmodus „$mode“ wird nicht unterstützt (kein Anhängen)")
        val handle = backend.open(checked(documentId), mode)
        if (handle < 0) throw FileNotFoundException("Öffnen fehlgeschlagen (errno ${-handle})")
        val app = context!!
        val thread = HandlerThread("se-doc-$handle").apply { start() }
        val callback = ProxyFileCallback(backend, handle, thread) {
            if (parsed.write) DocsCommitWorker.enqueue(app)
        }
        return try {
            app.getSystemService(StorageManager::class.java)
                .openProxyFileDescriptor(ParcelFileDescriptor.parseMode(mode), callback, Handler(thread.looper))
        } catch (e: IOException) {
            backend.release(handle)
            thread.quitSafely()
            throw FileNotFoundException("Kein Dateideskriptor: ${e.javaClass.simpleName}")
        }
    }

    override fun createDocument(parentDocumentId: String, mimeType: String, displayName: String): String =
        core("createDocument") {
            backend.create(checked(parentDocumentId), displayName, mimeType == Document.MIME_TYPE_DIR).documentId
        }

    override fun renameDocument(documentId: String, displayName: String): String =
        core("renameDocument") { backend.rename(checked(documentId), displayName) }

    override fun deleteDocument(documentId: String) {
        core("deleteDocument") { backend.delete(checked(documentId)) }
    }

    private fun checked(id: String): String =
        id.takeIf { DocumentIds.valid(it) } ?: throw FileNotFoundException("Unbekanntes Dokument")

    /** Runs a core call; every failure reaches the caller as [FileNotFoundException]. */
    private inline fun <T> core(name: String, work: () -> T): T = try {
        work()
    } catch (e: FileNotFoundException) {
        throw e
    } catch (e: CoreException) {
        Log.w(TAG, "$name: ${e.kind}")
        throw FileNotFoundException(e.message ?: e.kind)
    } catch (e: Exception) {
        Log.w(TAG, "$name: ${e.javaClass.simpleName}")
        throw FileNotFoundException("Smart Explorer antwortet nicht")
    }

    companion object {
        private const val TAG = "SmartExplorerDocs"
        private val watching = AtomicBoolean(false)

        /** The core behind the provider; the Robolectric tests pass their own. */
        @VisibleForTesting
        var backendFactory: () -> DocsBackend = { NativeDocsBackend() }

        /** Starts the core (idempotent); the Robolectric tests record the call instead. */
        @VisibleForTesting
        var startCore: (Application) -> Unit = { Core.start(it) }

        /** Shows or hides every root for other apps (setting „Ordner für Apps bereitstellen“). */
        fun setEnabled(context: Context, enabled: Boolean) {
            val component = ComponentName(context, SeDocumentsProvider::class.java)
            val state = if (enabled) PackageManager.COMPONENT_ENABLED_STATE_ENABLED else PackageManager.COMPONENT_ENABLED_STATE_DISABLED
            if (context.packageManager.getComponentEnabledSetting(component) != state) {
                context.packageManager.setComponentEnabledSetting(component, state, PackageManager.DONT_KILL_APP)
            }
            context.contentResolver.notifyChange(DocumentsContract.buildRootsUri(DocumentIds.authority(context)), null)
        }
    }
}
