package app.smartexplorer.android.ui.screen

import androidx.compose.runtime.staticCompositionLocalOf
import app.smartexplorer.android.api.DecoderOffer
import app.smartexplorer.android.api.ScreenApi
import app.smartexplorer.android.api.ScreenBufferBudget
import app.smartexplorer.android.api.ScreenDevice
import app.smartexplorer.android.api.ScreenProbe
import app.smartexplorer.android.api.ScreenInputCaps
import app.smartexplorer.android.api.ScreenMonitor
import app.smartexplorer.android.api.ScreenWindows
import app.smartexplorer.android.api.FamilyState
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.ScreenViewEvent
import app.smartexplorer.android.screen.ScreenIconRequest
import app.smartexplorer.android.screen.ScreenIconReply
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
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.intOrNull
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
    val frameContract: Boolean = false,
    val bufferBudget:ScreenBufferBudget? = null,
    val inputReady: Boolean = true,
    val path: String? = null,
    val platform: String? = null,
    val view: String = "off",
    val control: String = "off",
    val input: ScreenInputCaps = ScreenInputCaps(),
    val monitors: List<ScreenMonitor> = emptyList(),
    val limits: List<String> = emptyList(),
    /** The newest picture group. */
    val stream: ScreenViewEvent.Stream? = null,
    /** The latest groups by their full native GOP ID, for the decoder. */
    val streams: Map<Long, ScreenViewEvent.Stream> = emptyMap(),
    val windows: ScreenWindows? = null,
    /** `viewer`, `other` or `null`. */
    val holder: String? = null,
    val blocked: String? = null,
    val errorCode: String? = null,
    val errorDetail: String? = null,
    /** An automatic codec fallback, separate from a terminal session error. */
    val codecFallback: String? = null,
    val stats: JsonObject? = null,
    val ended: ScreenViewEvent.Ended? = null,
    val iconPeer: String = "",
    val iconFetch: (suspend (ScreenIconRequest) -> ScreenIconReply?)? = null,
    val iconVersion: Int? = null,
    val family: FamilyState? = null,
) {
    /** The host lets this session control and the device can take input of some kind. */
    val canControl: Boolean
        get() = accepted && control != "off" && (input.pointer || input.touch || input.keyboard || input.text)

    /** The state after [event]. */
    fun apply(event: ScreenViewEvent): ScreenSessionState = when (event) {
        is ScreenViewEvent.Pending -> copy(pending = event.capability, state = "waiting_for_approval")
        is ScreenViewEvent.Accepted -> copy(
            iconVersion = event.body["iconVersion"]?.toString()?.toIntOrNull(),
            accepted = true,
            frameContract = ((event.body["frameContract"] ?: event.body["frame_contract"]) as? JsonObject)?.get("version")?.jsonPrimitive?.intOrNull == 2,
            bufferBudget = event.body["bufferBudget"]?.let {
                runCatching {screenJson.decodeFromJsonElement<ScreenBufferBudget>(it)}.getOrNull()
            } ?: if((event.body["frameContract"] ?: event.body["frame_contract"]) !is JsonObject) ScreenBufferBudget.LEGACY else null,
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
        is ScreenViewEvent.VisibilityAck -> this
        is ScreenViewEvent.State -> copy(state=event.state,reason=event.reason,
            codecFallback=event.reason?.takeIf {it.startsWith("encoder_fallback_to_tiles: ")}
                ?.removePrefix("encoder_fallback_to_tiles: ")?.let {"Videocodec-Fallback: Verlustfreie Übertragung gewählt. $it"} ?: codecFallback)
        is ScreenViewEvent.Stream -> copy(
            stream = event,
            streams = (streams + (event.gop to event)).let { all ->
                // The decoder only looks up the groups of the pictures it holds.
                if (all.size <= STREAM_CACHE) all else all.entries.drop(all.size - STREAM_CACHE).associate { it.key to it.value }
            },
        )
        is ScreenViewEvent.Stats -> copy(stats = event.body)
        is ScreenViewEvent.ControlState -> copy(holder = event.holder, blocked = event.blocked)
        is ScreenViewEvent.Windows -> copy(windows = runCatching { screenJson.decodeFromJsonElement<ScreenWindows>(event.body) }.getOrNull())
        is ScreenViewEvent.Family -> copy(family = screenJson.decodeFromJsonElement<FamilyState>(event.body))
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

    suspend fun icon(session: Long, request: ScreenIconRequest): ScreenIconReply? = null

    suspend fun keyframe(session: Long)

    suspend fun visibility(session: Long, visibility: ScreenVisibilityReport) { }

    suspend fun presented(session: Long, gopId: Long, frameId: Long) { }

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
        ScreenApi.viewOpen(target.relation, target.relationId, target.deviceId, mode, source, decoders, displayWidth, displayHeight, maxFps, ScreenScaling.awaitRendererLimit(), target.expectedNodeId)

    override suspend fun setMode(session: Long, mode: String, source: String?, width: Int?, height: Int?, fps: Int?, sharpness: Boolean, lossless: Boolean) =
        ScreenApi.viewSetMode(session, mode, source, width, height, fps, sharpness, lossless)

    override suspend fun input(session: Long, events: List<JsonObject>) = ScreenApi.viewInput(session, events)

    override suspend fun control(session: Long, action: String) = ScreenApi.viewControl(session, action)

    override suspend fun listWindows(session: Long) = ScreenApi.viewListWindows(session)

    override suspend fun icon(session: Long, request: ScreenIconRequest) = ScreenApi.viewIcon(session, request)

    override suspend fun keyframe(session: Long) = ScreenApi.viewKeyframe(session)

    override suspend fun visibility(session: Long, visibility: ScreenVisibilityReport) = ScreenApi.viewVisibility(session, visibility.asJson())

    override suspend fun presented(session: Long, gopId: Long, frameId: Long) = ScreenApi.viewPresented(session, gopId, frameId)

    override suspend fun close(session: Long) = ScreenApi.viewClose(session)
}

/** The backend the screens below use. */
val LocalScreenBackend = staticCompositionLocalOf<ScreenBackend> { CoreScreenBackend }

/**
 * One viewer session of the core as the screens use it: opens it, follows its events, sends input
 * in order and closes it. The events are collected from before `viewOpen` is called, so none of
 * the first ones is lost.
 */
private sealed interface ScreenSessionCommand {
    data class Input(val events: List<JsonObject>) : ScreenSessionCommand
    data class Visibility(val report: ScreenVisibilityReport) : ScreenSessionCommand
    data class Presented(val frame: ScreenPresentedFrame) : ScreenSessionCommand
    data class Call(val action: suspend ScreenBackend.(Long) -> Unit) : ScreenSessionCommand
}

private data class ScreenModeRequest(val mode:String="full",
    val sharpness:Boolean=false,val lossless:Boolean=false)

/** Identity of the complete image actually consumed by a Bitmap or Surface draw. */
data class ScreenPresentedFrame(val session: Long, val gopId: Long, val frameId: Long, val epoch: Long)

class ScreenSession(private val target: ScreenTarget, private val backend: ScreenBackend = CoreScreenBackend) {
    val familyRequired: Boolean = target.source?.startsWith("window:@family.") == true
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val mutable = MutableStateFlow(ScreenSessionState(
        iconPeer = listOf(target.relation, target.relationId, target.deviceId.orEmpty()).joinToString("") { "${it.length}:$it" },
        iconFetch = { request -> if (id == 0L) null else backend.icon(id, request) },
    ))
    private val lock = Any()
    private var collector: Job? = null
    private val inputs = Channel<ScreenSessionCommand>(Channel.UNLIMITED)
    private var visibilityFacts: ScreenVisibilityFacts? = null
    private var visibilityReport: ScreenVisibilityReport? = null
    private var visibilitySentModern = false
    private var minFrame: Long? = null
    private var latestComplete: Long? = null
    private var displayEpoch = 0L
    private var rejectedGop: Long? = null
    private var codecFallbackGop:Long?=null
    private var forceLossless=false
    private var bitmapPending=false
    /** Survives Surface disposal until this viewer session itself is released. */
    internal val decoderBuffers=ScreenPullBuffers {mutable.value.bufferBudget}
    internal val scaleWorker=ScreenScaleWorker {mutable.value.bufferBudget}
    private var wantedMode=ScreenModeRequest()
    private val commands=object:ScreenBackend by backend {
        override suspend fun setMode(session:Long,mode:String,source:String?,width:Int?,height:Int?,fps:Int?,sharpness:Boolean,lossless:Boolean) {
            val forced=synchronized(lock) {forceLossless}
            backend.setMode(session,mode,source,width,height,fps,sharpness,lossless || forced)
            synchronized(lock) {
                wantedMode=ScreenModeRequest(mode,sharpness,lossless)
            }
        }
    }
    private var lastPresented: ScreenPresentedFrame? = null
    private var closed = false
    private var inputJob: Job? = null

    /** The core's session number; 0 before [open] returned. */
    @Volatile var id: Long = 0
        private set

    val state: StateFlow<ScreenSessionState> = mutable.asStateFlow()

    val deviceTarget: ScreenTarget get() = target

    /** Opens the session; a failure arrives as [CoreException] and leaves the session closed. */
    suspend fun open(mode: String, source: String?, displayWidth: Int, displayHeight: Int, maxFps: Int, decoders: List<DecoderOffer>) {
        synchronized(lock) {wantedMode=ScreenModeRequest(if(mode=="preview") "preview" else "full")}
        val backlog = ArrayList<ScreenViewEvent>()
        collector = scope.launch(start = CoroutineStart.UNDISPATCHED) {
            backend.events.collect { event ->
                synchronized(lock) {
                    if (id == 0L) backlog.add(event) else if (event.session == id) applyEvent(event)
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
            backlog.filter { it.session == opened }.forEach { applyEvent(it) }
            backlog.clear()
        }
        inputJob = scope.launch {
            for (command in inputs) {
                try {
                    when (command) {
                        is ScreenSessionCommand.Input -> if (!familyRequired || state.value.inputReady) {
                            command.events.chunked(MAX_EVENTS_PER_CALL).forEach { backend.input(opened, it) }
                        }
                        is ScreenSessionCommand.Visibility -> backend.visibility(opened, command.report)
                        is ScreenSessionCommand.Presented -> {
                            val frame = command.frame
                            if (isDisplayable(frame) && state.value.frameContract) backend.presented(opened, frame.gopId, frame.frameId)
                        }
                        is ScreenSessionCommand.Call -> command.action.invoke(commands, opened)
                    }
                } catch (e: CoreException) {
                    synchronized(lock) { mutable.value = mutable.value.copy(errorCode = e.kind, errorDetail = e.message) }
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
        if (events.isNotEmpty()) inputs.trySend(ScreenSessionCommand.Input(events))
    }

    private fun applyEvent(event: ScreenViewEvent) {
        val old = mutable.value
        if(event is ScreenViewEvent.State && event.reason?.startsWith("encoder_fallback_to_tiles: ")==true) forceLossless=true
        val familyLayoutChanged = event is ScreenViewEvent.Family &&
            screenJson.decodeFromJsonElement<FamilyState>(event.body).snapshot?.layout_revision != old.family?.snapshot?.layout_revision
        if (event is ScreenViewEvent.Stream && old.stream?.gop != event.gop ||
            event is ScreenViewEvent.State && event.state != old.state && event.state != "streaming" ||
            event is ScreenViewEvent.Ended || familyLayoutChanged) {
            displayEpoch++
            latestComplete = null
            lastPresented = null
            mutable.value = old.copy(inputReady = false)
        }
        mutable.value = mutable.value.apply(event)
        if (event is ScreenViewEvent.VisibilityAck && event.geometryGeneration == visibilityReport?.geometryGeneration) {
            minFrame = event.minFrameId
            mutable.value = mutable.value.copy(inputReady = visibilityReport?.state != "hidden" &&
                latestComplete?.let { it >= event.minFrameId } == true)
        }
        val report = visibilityReport
        if (mutable.value.frameContract && !visibilitySentModern && report != null) {
            visibilitySentModern = true
            minFrame = null
            latestComplete = null
            mutable.value = mutable.value.copy(inputReady = false)
            inputs.trySend(ScreenSessionCommand.Visibility(report))
        }
    }

    /** Release events are queued before this report by the view on hiding. */
    internal fun publishVisibility(facts: ScreenVisibilityFacts) = synchronized(lock) {
        if (visibilityFacts == facts) return@synchronized
        visibilityFacts = facts
        val rects = facts.rects()
        val state = if (!facts.visible) "hidden" else if (facts.sourceWidth <= 0 || facts.sourceHeight <= 0) "unknown" else if (rects.isEmpty()) "hidden" else "visible"
        val report = ScreenVisibilityReport((visibilityReport?.geometryGeneration ?: 0L)+1L, state, false, rects)
        visibilityReport = report
        displayEpoch++
        minFrame = null
        latestComplete = null
        lastPresented = null
        mutable.value = mutable.value.copy(inputReady = false)
        if (mutable.value.frameContract) {
            visibilitySentModern = true
            inputs.trySend(ScreenSessionCommand.Visibility(report))
        }
    }

    internal fun invalidateDisplay(requireNewGop: Boolean = true) = synchronized(lock) {
        displayEpoch++
        if (requireNewGop) rejectedGop = mutable.value.stream?.gop
        visibilityFacts = null
        latestComplete = null
        lastPresented = null
        mutable.value = mutable.value.copy(inputReady = false)
    }

    fun presentationEpoch(): Long = synchronized(lock) { displayEpoch }
    /** Shared by decoder/Surface handovers: one immutable snapshot may wait on Main. */
    fun beginBitmapSnapshot():Boolean = synchronized(lock) {
        if(closed || bitmapPending) false else {bitmapPending=true;true}
    }
    fun endBitmapSnapshot() = synchronized(lock) {bitmapPending=false}

    fun isDisplayable(frame: ScreenPresentedFrame): Boolean = synchronized(lock) {
        val current = mutable.value
        !closed && frame.session == id && frame.epoch == displayEpoch && current.ended == null &&
            current.accepted && current.state in listOf("streaming", "starting") &&
            frame.gopId == current.stream?.gop && frame.gopId != rejectedGop && visibilityReport?.state != "hidden" &&
            (!forceLossless || current.stream?.codec != "h264") &&
            (!current.frameContract || minFrame?.let { frame.frameId >= it } == true)
            && (!familyRequired || current.family?.snapshot?.layout_revision?.let {
                current.stream?.familyLayoutRevision == it
            } == true)
    }

    /** Only renderer callbacks invoke this; queued decoder and scaler output cannot open input. */
    fun confirmPresented(frame: ScreenPresentedFrame) = synchronized(lock) {
        if (!isDisplayable(frame)) return@synchronized
        latestComplete = maxOf(latestComplete ?: 0L, frame.frameId)
        if (mutable.value.frameContract && (lastPresented?.let {
            it.epoch == frame.epoch && it.gopId == frame.gopId && it.frameId >= frame.frameId
        } != true)) {
            lastPresented = frame
            if (inputs.trySend(ScreenSessionCommand.Presented(frame)).isFailure) return@synchronized
        }
        mutable.value = mutable.value.copy(inputReady = true)
    }

    /** A decoder failure remains valid when queue recovery has already announced a newer GOP. */
    fun hybridUnavailable(gopId: Long, reason: String, mode: String = "full") = synchronized(lock) {
        val current=mutable.value
        if(closed || id==0L || current.ended!=null || forceLossless ||
            !current.streams.containsKey(gopId) || codecFallbackGop==gopId) return@synchronized
        if(mode!="full" && mode!="preview") return@synchronized
        codecFallbackGop=gopId;forceLossless=true;rejectedGop=current.stream?.gop;displayEpoch++
        latestComplete=null;lastPresented=null
        mutable.value=mutable.value.copy(inputReady=false,
            codecFallback="Video-Einzelinputausgabe nicht bestätigt. Verlustfreie Übertragung angefordert. ${reason.take(768)}")
        inputs.trySend(ScreenSessionCommand.Call {session->
            val wanted=synchronized(lock) {wantedMode}
            // The host owns the separate full/preview size and rate choices.
            // A codec fallback changes none of those choices.
            backend.setMode(session,wanted.mode,source=null,width=null,height=null,
                fps=null,sharpness=wanted.sharpness,lossless=true)
        })
    }

    /** Quality changes preserve omitted preferences and a session's proven codec fallback. */
    fun setMode(mode:String,source:String?=null,width:Int?=null,height:Int?=null,fps:Int?=null,
        sharpness:Boolean?=null,lossless:Boolean?=null) {
        call {session->
            val wanted=synchronized(lock) {wantedMode}
            setMode(session,mode,source,width,height,fps,sharpness ?: wanted.sharpness,lossless ?: wanted.lossless)
        }
    }

    /** Runs a call on this session in the background; its failure becomes the shown error. */
    fun call(block: suspend ScreenBackend.(Long) -> Unit) {
        val session = id
        if (session == 0L) return
        inputs.trySend(ScreenSessionCommand.Call(block))
    }

    /** Closes the session in the core once the queued input has gone out; closing twice is fine. */
    fun close() {
        synchronized(lock) { closed = true; displayEpoch++; latestComplete = null; mutable.value = mutable.value.copy(inputReady = false) }
        scaleWorker.close()
        ScreenIconContent.forgetPeer(mutable.value.iconPeer)
        collector?.cancel()
        inputs.close()
        if (id == 0L) scope.cancel()
    }

    companion object {
        /** `MAX_INPUT_EVENTS` of the core: more events in one call are refused. */
        const val MAX_EVENTS_PER_CALL = 256

    }
}
