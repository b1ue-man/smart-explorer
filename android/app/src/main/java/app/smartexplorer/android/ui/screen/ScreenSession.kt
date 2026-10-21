package app.smartexplorer.android.ui.screen

import androidx.compose.runtime.staticCompositionLocalOf
import app.smartexplorer.android.api.DecoderOffer
import app.smartexplorer.android.api.ScreenApi
import app.smartexplorer.android.api.ScreenDevice
import app.smartexplorer.android.api.ScreenProbe
import app.smartexplorer.android.api.ScreenInputCaps
import app.smartexplorer.android.api.ScreenMonitor
import app.smartexplorer.android.api.ScreenWindows
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.ScreenViewEvent
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.filterIsInstance
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.decodeFromJsonElement

private val screenJson = Json {
    ignoreUnknownKeys = true
    coerceInputValues = true
}

/** What the app knows of one viewer session; the screens only show it (SCR-M9). */
data class ScreenSessionState(
    val state: String = "connecting",
    val reason: String? = null,
    /** The device asks its user (`screenView` or `screenControl`). */
    val pending: String? = null,
    val accepted: Boolean = false,
    val path: String? = null,
    val platform: String? = null,
    val view: String = "off",
    val control: String = "off",
    val input: ScreenInputCaps = ScreenInputCaps(),
    val monitors: List<ScreenMonitor> = emptyList(),
    val limits: List<String> = emptyList(),
    /** The newest picture group. */
    val stream: ScreenViewEvent.Stream? = null,
    /** The latest groups by the low 16 bits of their number, for the decoder. */
    val streams: Map<Int, ScreenViewEvent.Stream> = emptyMap(),
    val windows: ScreenWindows? = null,
    /** `viewer`, `other` or `null`. */
    val holder: String? = null,
    val blocked: String? = null,
    val errorCode: String? = null,
    val errorDetail: String? = null,
    val stats: JsonObject? = null,
    val ended: ScreenViewEvent.Ended? = null,
) {
    /** The host lets this session control and the device can take input of some kind. */
    val canControl: Boolean
        get() = accepted && control != "off" && (input.pointer || input.touch || input.keyboard || input.text)

    /** The state after [event]. */
    fun apply(event: ScreenViewEvent): ScreenSessionState = when (event) {
        is ScreenViewEvent.Pending -> copy(pending = event.capability, state = "waiting_for_approval")
        is ScreenViewEvent.Accepted -> copy(
            accepted = true,
            pending = null,
            path = event.path,
            platform = event.platform,
            view = event.view,
            control = event.control,
            input = event.body["input"]?.let { screenJson.decodeFromJsonElement<ScreenInputCaps>(it) } ?: ScreenInputCaps(),
            monitors = (event.body["monitors"] as? JsonArray)
                ?.map { screenJson.decodeFromJsonElement<ScreenMonitor>(it) }.orEmpty(),
            limits = (event.body["limits"] as? JsonArray)?.map { it.toString().trim('"') }.orEmpty(),
            state = if (state == "connecting" || state == "waiting_for_approval") "starting" else state,
        )
        is ScreenViewEvent.State -> copy(state = event.state, reason = event.reason)
        is ScreenViewEvent.Stream -> copy(
            stream = event,
            streams = (streams + ((event.gop and 0xFFFF).toInt() to event)).let { all ->
                // The decoder only looks up the groups of the pictures it holds.
                if (all.size <= STREAM_CACHE) all else all.entries.drop(all.size - STREAM_CACHE).associate { it.key to it.value }
            },
        )
        is ScreenViewEvent.Stats -> copy(stats = event.body)
        is ScreenViewEvent.ControlState -> copy(holder = event.holder, blocked = event.blocked)
        is ScreenViewEvent.Windows -> copy(windows = runCatching { screenJson.decodeFromJsonElement<ScreenWindows>(event.body) }.getOrNull())
        is ScreenViewEvent.Error -> copy(errorCode = event.code, errorDetail = event.detail)
        is ScreenViewEvent.Ended -> copy(ended = event, state = "ended")
    }

    companion object {
        /** Groups kept for the decoder (a group lives as long as its stream; this is a memory bound). */
        const val STREAM_CACHE = 16
    }
}

/**
 * Everything the screens ask of the core (SCR-M9); the screens take it from [LocalScreenBackend], so a
 * test can put a fake in its place. [CoreScreenBackend] is the real one.
 */
interface ScreenBackend {
    /** The `screenView` events of all sessions. */
    val events: Flow<ScreenViewEvent>

    fun offers(): List<DecoderOffer>

    /** The pictures of session [session]. */
    fun pictures(session: Long): PictureSource

    suspend fun devices(roomId: String?): List<ScreenDevice>

    suspend fun probe(target: ScreenTarget): ScreenProbe

    suspend fun open(
        target: ScreenTarget,
        mode: String,
        source: String?,
        displayWidth: Int,
        displayHeight: Int,
        maxFps: Int,
        decoders: List<DecoderOffer>,
    ): Long

    suspend fun setMode(session: Long, mode: String, source: String? = null, width: Int? = null, height: Int? = null, fps: Int? = null, sharpness: Boolean = false, lossless: Boolean = false)

    suspend fun input(session: Long, events: List<JsonObject>): Int

    suspend fun control(session: Long, action: String)

    suspend fun listWindows(session: Long)

    suspend fun keyframe(session: Long)

    suspend fun close(session: Long)
}

/** The core: `screen.*` of the bridge and `screenPull`. */
object CoreScreenBackend : ScreenBackend {
    override val events: Flow<ScreenViewEvent> get() = Core.events.filterIsInstance<ScreenViewEvent>()

    override fun offers() = ScreenDecoder.offers()

    override fun pictures(session: Long): PictureSource = NativePictureSource(session)

    override suspend fun devices(roomId: String?) = ScreenApi.devices(roomId)

    override suspend fun probe(target: ScreenTarget) = ScreenApi.probe(target.relation, target.relationId, target.deviceId)

    override suspend fun open(target: ScreenTarget, mode: String, source: String?, displayWidth: Int, displayHeight: Int, maxFps: Int, decoders: List<DecoderOffer>) =
        ScreenApi.viewOpen(target.relation, target.relationId, target.deviceId, mode, source, decoders, displayWidth, displayHeight, maxFps)

    override suspend fun setMode(session: Long, mode: String, source: String?, width: Int?, height: Int?, fps: Int?, sharpness: Boolean, lossless: Boolean) =
        ScreenApi.viewSetMode(session, mode, source, width, height, fps, sharpness, lossless)

    override suspend fun input(session: Long, events: List<JsonObject>) = ScreenApi.viewInput(session, events)

    override suspend fun control(session: Long, action: String) = ScreenApi.viewControl(session, action)

    override suspend fun listWindows(session: Long) = ScreenApi.viewListWindows(session)

    override suspend fun keyframe(session: Long) = ScreenApi.viewKeyframe(session)

    override suspend fun close(session: Long) = ScreenApi.viewClose(session)
}

/** The backend the screens below use. */
val LocalScreenBackend = staticCompositionLocalOf<ScreenBackend> { CoreScreenBackend }

/**
 * One viewer session of the core as the screens use it: opens it, follows its events, sends input
 * in order and closes it. The events are collected from before `viewOpen` is called, so none of
 * the first ones is lost.
 */
class ScreenSession(private val target: ScreenTarget, private val backend: ScreenBackend = CoreScreenBackend) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val mutable = MutableStateFlow(ScreenSessionState())
    private val lock = Any()
    private var collector: Job? = null
    private val inputs = Channel<List<JsonObject>>(Channel.UNLIMITED)
    private var inputJob: Job? = null

    /** The core's session number; 0 before [open] returned. */
    @Volatile var id: Long = 0
        private set

    val state: StateFlow<ScreenSessionState> = mutable.asStateFlow()

    val deviceTarget: ScreenTarget get() = target

    /** Opens the session; a failure arrives as [CoreException] and leaves the session closed. */
    suspend fun open(mode: String, source: String?, displayWidth: Int, displayHeight: Int, maxFps: Int, decoders: List<DecoderOffer>) {
        val backlog = ArrayList<ScreenViewEvent>()
        collector = scope.launch(start = CoroutineStart.UNDISPATCHED) {
            backend.events.collect { event ->
                synchronized(lock) {
                    if (id == 0L) backlog.add(event) else if (event.session == id) mutable.value = mutable.value.apply(event)
                }
            }
        }
        val opened = try {
            backend.open(target, mode, source, displayWidth, displayHeight, maxFps, decoders)
        } catch (e: CoreException) {
            collector?.cancel()
            throw e
        }
        synchronized(lock) {
            id = opened
            backlog.filter { it.session == opened }.forEach { mutable.value = mutable.value.apply(it) }
            backlog.clear()
        }
        inputJob = scope.launch {
            for (batch in inputs) {
                try {
                    batch.chunked(MAX_EVENTS_PER_CALL).forEach { backend.input(opened, it) }
                } catch (e: CoreException) {
                    mutable.value = mutable.value.copy(errorCode = e.kind, errorDetail = e.message)
                }
            }
            // The channel was closed: what was queued (the release of pressed keys) has gone out.
            try {
                backend.close(opened)
            } catch (e: CoreException) {
                Unit
            }
            scope.cancel()
        }
    }

    /** Queues input events; they reach the core in the order they were queued. */
    fun sendInput(events: List<JsonObject>) {
        if (events.isNotEmpty()) inputs.trySend(events)
    }

    /** Runs a call on this session in the background; its failure becomes the shown error. */
    fun call(block: suspend ScreenBackend.(Long) -> Unit) {
        val session = id
        if (session == 0L) return
        scope.launch {
            try {
                backend.block(session)
            } catch (e: CoreException) {
                mutable.value = mutable.value.copy(errorCode = e.kind, errorDetail = e.message)
            }
        }
    }

    /** Closes the session in the core once the queued input has gone out; closing twice is fine. */
    fun close() {
        collector?.cancel()
        inputs.close()
        if (id == 0L) scope.cancel()
    }

    companion object {
        /** `MAX_INPUT_EVENTS` of the core: more events in one call are refused. */
        const val MAX_EVENTS_PER_CALL = 256

    }
}
