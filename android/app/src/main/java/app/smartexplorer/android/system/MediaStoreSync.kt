package app.smartexplorer.android.system

import android.content.Context
import android.database.ContentObserver
import android.net.Uri
import android.os.Environment
import android.os.storage.StorageManager
import android.provider.MediaStore
import android.util.Log
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.service.BackgroundController
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.*

/** MediaStore observes part of shared storage; neither equal generations nor
 * a quiet observer certify an unchanged sync tree. Native polling stays on. */
internal object MediaStoreSync {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val lock = Mutex()
    private val previous = mutableMapOf<String, String>()
    private var observer: ContentObserver? = null
    private val signals = Channel<Context>(Channel.CONFLATED)

    init {
        scope.launch {
            for (context in signals) {
                delay(1_000)
                probe(context, signal = true)
                BackgroundController.enqueueCatchUp(context)
            }
        }
    }

    fun start(context: Context) {
        if (observer != null) return
        val app = context.applicationContext
        val contentObserver = object : ContentObserver(null) {
            override fun onChange(selfChange: Boolean) = signal(app)
            override fun onChange(selfChange: Boolean, uri: Uri?, flags: Int) = signal(app)
            override fun onChange(selfChange: Boolean, uris: Collection<Uri>, flags: Int) = signal(app)
        }
        try {
            app.contentResolver.registerContentObserver(MediaStore.Files.getContentUri(MediaStore.VOLUME_EXTERNAL), true, contentObserver)
            observer = contentObserver
        } catch (e: RuntimeException) { Log.w("SmartExplorerSync", "MediaStore-Hinweise fehlen; Abfragen bleiben aktiv", e) }
        scope.launch { probe(app) }
    }

    private fun signal(context: Context) {
        signals.trySend(context.applicationContext)
    }

    suspend fun probe(context: Context, signal: Boolean = false) = lock.withLock {
        val granted = Permissions.hasAllFilesAccess()
        val storage = context.getSystemService(StorageManager::class.java) ?: return@withLock
        val rows = storage.storageVolumes.mapNotNull { volume ->
            if (volume.state != Environment.MEDIA_MOUNTED && volume.state != Environment.MEDIA_MOUNTED_READ_ONLY) return@mapNotNull null
            val root = volume.directory?.absolutePath ?: return@mapNotNull null
            val name = volume.mediaStoreVolumeName
            val cursor = if (granted && name != null) try {
                // Read version on both sides: a DB reset during the read is an unknown hint.
                val version = MediaStore.getVersion(context, name)
                val generation = MediaStore.getGeneration(context, name)
                if (version == MediaStore.getVersion(context, name)) "$name:$version:$generation" else null
            } catch (e: RuntimeException) { null } else null
            val old = previous[root]
            if (cursor == null) previous.remove(root) else previous[root] = cursor
            buildJsonObject {
                put("path", root)
                cursor?.let { put("cursor", it) } ?: put("cursor", JsonNull)
                put("changed", signal || cursor == null || old != cursor)
            }
        }
        try { Core.call("sys.watchHints", buildJsonObject { put("volumes", JsonArray(rows)) }) }
        catch (e: Exception) {
            if (e is CancellationException) throw e
            Log.w("SmartExplorerSync", "MediaStore-Hinweise nicht übergeben", e)
        }
    }
}
