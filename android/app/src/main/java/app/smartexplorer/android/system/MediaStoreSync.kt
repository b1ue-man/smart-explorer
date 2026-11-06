package app.smartexplorer.android.system

import android.content.Context
import android.database.ContentObserver
import android.net.Uri
import android.os.Environment
import android.os.storage.StorageManager
import android.provider.BaseColumns
import android.provider.MediaStore
import android.util.Log
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.api.SyncApi.displayText
import app.smartexplorer.android.api.SyncJob
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.service.BackgroundController
import java.io.File
import java.io.IOException
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.*

/** MediaStore observes part of shared storage; neither equal generations nor
 * a quiet observer certify an unchanged sync tree. Native polling stays on.
 *
 * MediaStore reports the changes of every app (camera, downloads, messengers, screenshots). Only
 * those below the local roots of enabled real-time jobs matter here: the observer and the content
 * trigger ([BackgroundController.armContentWake]) exist only while such a root exists, a volume is
 * reported to the core as changed only when MediaStore shows a change below one of its roots, and
 * only such a change queues a catch-up run. Per root the number of MediaStore rows below it and
 * the generation it was last compared at are kept across process starts, so a deletion (fewer
 * rows) counts as well as an added or modified file (a row of a newer generation). */
internal object MediaStoreSync {
    private const val TAG = "SmartExplorerSync"

    /** Per root: "<MediaStore version>|<generation>|<rows below the root>" of the last comparison. */
    private const val PREFS = "media-store-roots"

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val lock = Mutex()
    private val started = AtomicBoolean(false)
    private val previous = mutableMapOf<String, String>()
    private var observer: ContentObserver? = null
    private val observerGuard = Any()
    private val signals = Channel<Context>(Channel.CONFLATED)
    private val rootRefreshes = Channel<Context>(Channel.CONFLATED)

    /** Canonical local roots of the enabled real-time jobs; `null` until the job list was read once. */
    @Volatile
    private var roots: List<String>? = null

    init {
        scope.launch {
            for (context in signals) {
                delay(1_000)
                if (probe(context, signal = true)) BackgroundController.enqueueCatchUp(context)
            }
        }
    }

    fun start(context: Context) {
        if (!started.compareAndSet(false, true)) return
        val app = context.applicationContext
        // The first value registers the observer while the roots are still unknown (as before);
        // the job list then narrows it. "Aus" removes it.
        scope.launch { AppPrefs.bgMode.collect { updateObserver(app) } }
        scope.launch {
            refreshRoots(app)
            probe(app)
            for (next in rootRefreshes) refreshRoots(next)
        }
    }

    /** A `jobs` event: the real-time roots are read again (bursts collapse into one reading). */
    fun jobsChanged(context: Context) {
        rootRefreshes.trySend(context.applicationContext)
    }

    /** The observer and the content trigger are wanted: background mode on and a real-time root (or none read yet). */
    fun contentWanted(): Boolean =
        AppPrefs.bgMode.value != BackgroundController.MODE_OFF && roots?.isNotEmpty() != false

    /** Reads the real-time roots again (from every `jobs` event); arms or cancels observer and content trigger. */
    suspend fun refreshRoots(context: Context) {
        val app = context.applicationContext
        val loaded = try {
            SyncApi.jobs()
                .filter { it.enabled && it.trigger == SyncJob.TRIGGER_REALTIME }
                .flatMap { listOf(it.source, it.target) }
                .mapNotNull { localRoot(it) }
                .distinct()
                .sorted()
        } catch (e: CoreException) {
            Log.w(TAG, "Echtzeit-Wurzeln nicht gelesen: ${e.kind}: ${e.displayText()}")
            return
        }
        val changed = loaded != roots
        roots = loaded
        updateObserver(app)
        if (changed) BackgroundController.armContentWake(app)
    }

    /**
     * A worker started by the content trigger: whether MediaStore shows a change below a
     * real-time root since the last comparison (a fresh process reads the roots first).
     */
    suspend fun contentChanged(context: Context): Boolean {
        if (roots == null) refreshRoots(context)
        return probe(context, signal = true)
    }

    private fun updateObserver(context: Context) {
        synchronized(observerGuard) { updateObserverLocked(context) }
    }

    private fun updateObserverLocked(context: Context) {
        val wanted = contentWanted()
        val current = observer
        if (wanted && current == null) {
            val contentObserver = object : ContentObserver(null) {
                override fun onChange(selfChange: Boolean) = signal(context)
                override fun onChange(selfChange: Boolean, uri: Uri?, flags: Int) = signal(context)
                override fun onChange(selfChange: Boolean, uris: Collection<Uri>, flags: Int) = signal(context)
            }
            try {
                context.contentResolver.registerContentObserver(MediaStore.Files.getContentUri(MediaStore.VOLUME_EXTERNAL), true, contentObserver)
                observer = contentObserver
            } catch (e: RuntimeException) { Log.w(TAG, "MediaStore-Hinweise fehlen; Abfragen bleiben aktiv", e) }
        } else if (!wanted && current != null) {
            context.contentResolver.unregisterContentObserver(current)
            observer = null
        }
    }

    private fun signal(context: Context) {
        signals.trySend(context.applicationContext)
    }

    /**
     * Sends every mounted volume's MediaStore cursor and whether it changed to the core
     * (`sys.watchHints`). [signal]: the observer or the content trigger reported a change, which
     * may be a deletion that leaves the generation unchanged. Returns whether a change below a
     * real-time root was seen (while the roots are unknown: any change, as before).
     */
    suspend fun probe(context: Context, signal: Boolean = false): Boolean = lock.withLock {
        val granted = Permissions.hasAllFilesAccess()
        val storage = context.getSystemService(StorageManager::class.java) ?: return@withLock false
        val known = roots
        var relevant = false
        val rows = storage.storageVolumes.mapNotNull { volume ->
            if (volume.state != Environment.MEDIA_MOUNTED && volume.state != Environment.MEDIA_MOUNTED_READ_ONLY) return@mapNotNull null
            val root = volume.directory?.absolutePath ?: return@mapNotNull null
            val name = volume.mediaStoreVolumeName
            var stamp: Pair<String, Long>? = null
            val cursor = if (granted && name != null) try {
                // Read version on both sides: a DB reset during the read is an unknown hint.
                val version = MediaStore.getVersion(context, name)
                val generation = MediaStore.getGeneration(context, name)
                if (version == MediaStore.getVersion(context, name)) {
                    stamp = version to generation
                    "$name:$version:$generation"
                } else null
            } catch (e: RuntimeException) { null } else null
            val old = previous[root]
            if (cursor == null) previous.remove(root) else previous[root] = cursor
            val below = known?.let { inVolumeForm(it, root) }
            val volumeStamp = stamp
            val changed = when {
                below == null -> signal || cursor != null && old != cursor
                below.isEmpty() -> false
                // No MediaStore comparison possible (no all-files access, a version that changed
                // during the read): only a reported change (observer, content trigger, volume
                // event) counts, unseen. A timer brings no new fact, so it reports none; the
                // daemon's own polling of the roots stays on for what MediaStore never shows.
                cursor == null || name == null || volumeStamp == null -> signal
                !signal && old == cursor -> false
                // Only a reported change moves the stored comparison on: the timed probe between
                // a change and its (debounced) observer signal must not take that change away.
                else -> rootsChanged(context, name, volumeStamp, below, store = signal)
            }
            if (changed && below?.isEmpty() != true) relevant = true
            buildJsonObject {
                put("path", root)
                cursor?.let { put("cursor", it) } ?: put("cursor", JsonNull)
                put("changed", changed)
            }
        }
        try { Core.call("sys.watchHints", buildJsonObject { put("volumes", JsonArray(rows)) }) }
        catch (e: Exception) {
            if (e is CancellationException) throw e
            Log.w(TAG, "MediaStore-Hinweise nicht übergeben", e)
        }
        relevant
    }

    /**
     * Compares each root of the volume [name] with its stored row count and generation and stores
     * the new ones ([store]). Changed: no stored comparison, another MediaStore version, another row
     * count (a file came or went) or a row below the root modified after the stored generation. A
     * failed query counts as changed (MediaStore cannot vouch for the root).
     */
    private fun rootsChanged(context: Context, name: String, stamp: Pair<String, Long>, roots: List<String>, store: Boolean): Boolean {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val (version, generation) = stamp
        val files = MediaStore.Files.getContentUri(name)
        var changed = false
        val editor = prefs.edit()
        for (root in roots) {
            val stored = prefs.getString(root, null)?.split('|')
            val storedVersion = stored?.getOrNull(0)
            val storedGeneration = stored?.getOrNull(1)?.toLongOrNull()
            val storedCount = stored?.getOrNull(2)?.toIntOrNull()
            val count = rowsBelow(context, files, root, null)
            if (count == null) {
                changed = true
                editor.remove(root)
                continue
            }
            val sameBase = storedVersion == version && storedGeneration != null && storedCount != null
            val newer = if (sameBase && count == storedCount) rowsBelow(context, files, root, storedGeneration) else null
            if (!sameBase || count != storedCount || newer == null || newer > 0) changed = true
            editor.putString(root, "$version|$generation|$count")
        }
        if (store) editor.apply()
        return changed
    }

    /**
     * The real-time roots on the volume mounted at [volumeRoot], written below that path as
     * MediaStore stores it (`_data`): a root given through another path of the same volume (its
     * canonical form) is moved onto [volumeRoot].
     */
    private fun inVolumeForm(roots: List<String>, volumeRoot: String): List<String> {
        val canonical = try { File(volumeRoot).canonicalPath } catch (e: IOException) { volumeRoot }
        return roots.mapNotNull { root ->
            listOf(volumeRoot, canonical).distinct().firstNotNullOfOrNull { base ->
                when {
                    root == base -> volumeRoot
                    root.startsWith("$base/") -> volumeRoot + root.removePrefix(base)
                    else -> null
                }
            }
        }.distinct()
    }

    /** MediaStore rows at or below [root] (with [after]: only those modified after that generation); `null` when the query failed. */
    private fun rowsBelow(context: Context, files: Uri, root: String, after: Long?): Int? {
        val escaped = root.replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_")
        @Suppress("DEPRECATION")
        val data = MediaStore.MediaColumns.DATA
        val selection = StringBuilder("($data = ? OR $data LIKE ? ESCAPE '\\')")
        val args = mutableListOf(root, "$escaped/%")
        if (after != null) {
            selection.append(" AND ${MediaStore.MediaColumns.GENERATION_MODIFIED} > ?")
            args.add(after.toString())
        }
        return try {
            context.contentResolver.query(files, arrayOf(BaseColumns._ID), selection.toString(), args.toTypedArray(), null)
                ?.use { it.count }
        } catch (e: RuntimeException) {
            Log.w(TAG, "MediaStore-Abgleich einer Echtzeit-Wurzel fehlgeschlagen: ${e.javaClass.simpleName}")
            null
        }
    }

    /** The canonical path of a local job root (`/sdcard/…` and `/storage/emulated/0/…` alike); `null` for remote places. */
    private fun localRoot(endpoint: String): String? {
        if (!StorageStatsAccess.isLocalPath(endpoint)) return null
        val file = File(endpoint.trim())
        return try {
            file.canonicalPath
        } catch (e: IOException) {
            file.absolutePath
        }.trimEnd('/').ifEmpty { "/" }
    }
}
