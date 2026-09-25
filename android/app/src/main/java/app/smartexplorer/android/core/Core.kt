package app.smartexplorer.android.core

import android.app.Application
import android.os.SystemClock
import android.util.Log
import app.smartexplorer.android.system.Storage
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.SerializationException
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.encodeToJsonElement

/**
 * Kotlin side of the core API (api.md): loads the native library, runs `init` off the main
 * thread, pumps events and offers suspending calls that wait until the core is ready.
 */
object Core {
    private const val TAG = "SmartExplorerCore"
    private const val LIBRARY = "smart_explorer_android"
    private const val POLL_TIMEOUT_MS = 1_000
    private const val POLL_RETRY_MS = 1_000L
    private const val EVENT_BUFFER = 256

    val json: Json = Json {
        ignoreUnknownKeys = true
        explicitNulls = false
        encodeDefaults = true
        coerceInputValues = true
    }

    private val eventFlow = MutableSharedFlow<CoreEvent>(
        extraBufferCapacity = EVENT_BUFFER,
        onBufferOverflow = BufferOverflow.DROP_OLDEST,
    )
    val events: SharedFlow<CoreEvent> = eventFlow.asSharedFlow()

    private val taskStore = TaskStore()
    val tasks: StateFlow<List<TaskInfo>> = taskStore.tasks

    private val readyState = MutableStateFlow<CoreState>(CoreState.Starting)
    val ready: StateFlow<CoreState> = readyState.asStateFlow()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val initRunning = AtomicBoolean(false)
    private val pumpStarted = AtomicBoolean(false)
    private val volumeWatchStarted = AtomicBoolean(false)

    /**
     * Returns immediately. Loads the library and runs `init` on its own thread, then starts the
     * event pump. Calling it again while starting or after success does nothing; after a failure
     * it retries.
     */
    fun start(app: Application) {
        if (readyState.value == CoreState.Ready || !initRunning.compareAndSet(false, true)) return
        readyState.value = CoreState.Starting
        thread(name = "core-init", isDaemon = true) { initialize(app) }
    }

    /** Calls a core method (api.md §4); waits for [ready]; throws [CoreException] on errors. */
    suspend fun call(method: String, args: JsonObject = JsonObject(emptyMap())): JsonElement {
        awaitReady()
        val raw = withContext(Dispatchers.IO) {
            try {
                NativeBridge.call(method, args.toString())
            } catch (e: LinkageError) {
                throw CoreException("internal", "Kernfunktion nicht verfügbar: ${e.message}")
            }
        }
        val result = unwrap(raw)
        if (method == "task.clear") refreshTasksLogged()
        return result
    }

    suspend inline fun <reified T> request(method: String, args: JsonObject = JsonObject(emptyMap())): T =
        decodeResult(method, call(method, args))

    suspend inline fun <reified A, reified T> request(method: String, args: A): T {
        val encoded = json.encodeToJsonElement<A>(args) as? JsonObject
            ?: throw CoreException("invalid", "Argumente für $method sind kein JSON-Objekt")
        return decodeResult(method, call(method, encoded))
    }

    /** Reloads [tasks] from `task.list` (done automatically after start and `task.clear`). */
    suspend fun refreshTasks() {
        val list = call("task.list")
        taskStore.replaceAll(decodeResult<List<TaskInfo>>("task.list", list))
    }

    @PublishedApi
    internal inline fun <reified T> decodeResult(method: String, element: JsonElement): T =
        try {
            json.decodeFromJsonElement<T>(element)
        } catch (e: IllegalArgumentException) {
            // SerializationException is an IllegalArgumentException.
            throw CoreException("internal", "Antwort von $method nicht lesbar: ${e.message}")
        }

    private suspend fun awaitReady() {
        val state = ready.first { it !is CoreState.Starting }
        if (state is CoreState.Failed) throw CoreException("not_initialized", state.message)
    }

    private fun unwrap(raw: String): JsonElement {
        val envelope = try {
            json.parseToJsonElement(raw) as? JsonObject
        } catch (e: SerializationException) {
            null
        } ?: throw CoreException("internal", "Ungültige Antwort des Kerns")
        envelope["ok"]?.let { return it }
        val error = envelope["err"] as? JsonObject
            ?: throw CoreException("internal", "Ungültige Antwort des Kerns")
        val kind = (error["kind"] as? JsonPrimitive)?.contentOrNull ?: "internal"
        val message = (error["message"] as? JsonPrimitive)?.contentOrNull ?: kind
        throw CoreException(kind, message)
    }

    private fun initialize(app: Application) {
        val failure = try {
            System.loadLibrary(LIBRARY)
            unwrap(NativeBridge.init(app, InitConfig.build(app).toString()))
            null
        } catch (e: CoreException) {
            e.message ?: e.kind
        } catch (e: LinkageError) {
            "Kernbibliothek konnte nicht geladen werden: ${e.message}"
        } catch (e: Exception) {
            "Kernstart fehlgeschlagen: ${e.message ?: e.javaClass.simpleName}"
        }
        if (failure != null) {
            Log.e(TAG, "core start failed: $failure")
            readyState.value = CoreState.Failed(failure)
            initRunning.set(false)
            return
        }
        readyState.value = CoreState.Ready
        if (pumpStarted.compareAndSet(false, true)) {
            thread(name = "core-events", isDaemon = true) { pumpEvents() }
        }
        if (volumeWatchStarted.compareAndSet(false, true)) {
            Storage.watchVolumes(app) { scope.launch { pushVolumes(app) } }
        }
        scope.launch { refreshTasksLogged() }
    }

    /** The triggering action already succeeded; a failed reload only leaves the list stale. */
    private suspend fun refreshTasksLogged() {
        try {
            refreshTasks()
        } catch (e: CoreException) {
            Log.w(TAG, "task.list failed: ${e.kind}: ${e.message}")
        }
    }

    /** Blocking poll loop on its own daemon thread; never touches the main thread. */
    private fun pumpEvents() {
        while (true) {
            val started = SystemClock.elapsedRealtime()
            val raw = try {
                NativeBridge.pollEvents(POLL_TIMEOUT_MS)
            } catch (e: LinkageError) {
                Log.e(TAG, "event pump stopped: native pollEvents missing", e)
                return
            }
            val batch = CoreEvents.parse(json, raw)
            batch?.forEach { event ->
                if (event is CoreEvent.Task) taskStore.upsert(event.task)
                eventFlow.tryEmit(event)
            }
            // Back off when the core reports an error or returns early without events, so a
            // misbehaving poll can never spin.
            val early = SystemClock.elapsedRealtime() - started < POLL_TIMEOUT_MS / 2
            if (batch == null || (batch.isEmpty() && early)) {
                try {
                    Thread.sleep(POLL_RETRY_MS)
                } catch (e: InterruptedException) {
                    return
                }
            }
        }
    }

    private suspend fun pushVolumes(app: Application) {
        val volumes = json.encodeToJsonElement(ListSerializer(VolumeInfo.serializer()), Storage.volumes(app))
        try {
            call("sys.volumes", buildJsonObject { put("volumes", volumes) })
        } catch (e: CoreException) {
            Log.w(TAG, "sys.volumes failed: ${e.kind}: ${e.message}")
        }
    }
}
