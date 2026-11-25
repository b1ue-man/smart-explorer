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
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.filterIsInstance
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.cancel
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
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
    /**
     * The first frame the host's last `VISIBILITY_ACK` admits. Part of the state so that the ack always changes it:
     * a picture kept before the ack (`awaitsAck`) is drawn by the recomposition that follows (Codex 3/10).
     */
    val ackedMinFrame: Long? = null,
    /**
     * The host's `VISIBILITY_ACK`s of this viewer's reports, counted: an ack that names the same first frame as the one
     * before leaves [ackedMinFrame] equal, and a picture kept for it (`awaitsAck`) was not drawn (round 12, B5).
     */
    val visibilityAcks: Int = 0,
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
    /** The host held input back without applying it (E2); the next applied batch clears it. */
    val held: String? = null,
    val errorCode: String? = null,
    val errorDetail: String? = null,
    /** An automatic codec fallback, separate from a terminal session error. */
    val codecFallback: String? = null,
    /** The codec of the session's first picture group (`tiles`: the host cannot switch it to video in the session). */
    val firstCodec: String? = null,
    /** The viewer's codec choice („Codec“) the host's codec reports are read against ([ScreenCodecNotices]). */
    val chosenCodec: String = ViewerCodecChoice.CODEC_AUTO,
    /** The chosen codec is not in force: the host or this phone cannot run it (a visible error with its reason). */
    val codecError: String? = null,
    /** The host's encoders (`encoders`/`encodersV2` of `accepted`, or its probe before the opening); `null` before. */
    val hostCodecs: List<HostCodec>? = null,
    /** Why this session does not run a stored „Codec“ or „Dekodierer“ ([ScreenCodecOffer], user rule of 2026-11-24). */
    val sessionNotes: List<String> = emptyList(),
    /** `virtualDisplay` of the `accepted` event: what the host did with „Monitor“/„Auflösung“; `null` = no report. */
    val virtualDisplay: JsonObject? = null,
    /** `chain.virtualDisplay` of the latest statistics: the host's display now (`VirtualDisplayReport::chain_key`). */
    val chainVirtualDisplay: String? = null,
    /** `chain.rateCeilingKbps` of the latest statistics: the „Höchste Datenrate“ the host applies, in kbit/s. */
    val chainRateCeilingKbps: Long? = null,
    val stats: JsonObject? = null,
    /** „Übertragung“ the host runs and the rate it measured, from the latest statistics ([DeliveryReport]). */
    val delivery: DeliveryReport? = null,
    /** `rateCeiling` of the `accepted` event: `automatic`, `applied` or `unsupported_by_host` (B-09); `null` before it. */
    val rateCeiling: String? = null,
    /** Limits sent during the session ([ScreenSession.setMaxRate]) and answers to them (`rateCeiling` events, r1). */
    val rateSends: Int = 0,
    val rateAnswers: Int = 0,
    val ended: ScreenViewEvent.Ended? = null,
    val iconPeer: String = "",
    val iconFetch: (suspend (ScreenIconRequest) -> ScreenIconReply?)? = null,
    val iconVersion: Int? = null,
    val family: FamilyState? = null,
) {
    /** The host lets this session control and the device can take input of some kind. */
    val canControl: Boolean
        get() = accepted && control != "off" && (input.pointer || input.touch || input.keyboard || input.text)

    /** [rateCeiling] answers the limit chosen last: every limit sent during the session got its answer (r1). */
    val rateAnswered: Boolean
        get() = rateSends > 0 && rateAnswers >= rateSends

    /** The state after [event]. */
    fun apply(event: ScreenViewEvent): ScreenSessionState = when (event) {
        is ScreenViewEvent.Pending -> copy(pending = event.capability, state = "waiting_for_approval")
        is ScreenViewEvent.Accepted -> copy(
            iconVersion = event.body["iconVersion"]?.toString()?.toIntOrNull(),
            accepted = true,
            rateCeiling = (event.body["rateCeiling"] as? JsonPrimitive)?.takeIf { it.isString }?.content,
            frameContract = ((event.body["frameContract"] ?: event.body["frame_contract"]) as? JsonObject)?.get("version")?.jsonPrimitive?.intOrNull == 2,
            bufferBudget = event.body["bufferBudget"]?.let {
                runCatching {screenJson.decodeFromJsonElement<ScreenBufferBudget>(it)}.getOrNull()
            } ?: if(event.body["bufferBudget"]==null &&
                (event.body["frameContract"] ?: event.body["frame_contract"]) !is JsonObject) ScreenBufferBudget.LEGACY else null,
            pending = null,
            path = event.path,
            platform = event.platform,
            view = event.view,
            control = event.control,
            input = event.body["input"]?.let { screenJson.decodeFromJsonElement<ScreenInputCaps>(it) } ?: ScreenInputCaps(),
            monitors = (event.body["monitors"] as? JsonArray)
                ?.map { screenJson.decodeFromJsonElement<ScreenMonitor>(it) }.orEmpty(),
            limits = (event.body["limits"] as? JsonArray)?.map { it.toString().trim('"') }.orEmpty(),
            virtualDisplay = event.body["virtualDisplay"] as? JsonObject,
            hostCodecs = ScreenCodecOffer.hostCodecs(event.body, "encoders", "encodersV2") ?: hostCodecs,
            state = if (state == "connecting" || state == "waiting_for_approval") "starting" else state,
        )
        is ScreenViewEvent.VisibilityAck -> this
        is ScreenViewEvent.State -> ScreenCodecNotices.parse(event.reason, chosenCodec).let { notice ->
            // Every `encoder_fallback_to_<codec>` shows chosen against running; a codec the user named that the host left is an error.
            copy(state = event.state, reason = event.reason,
                codecFallback = notice?.takeIf { !it.error }?.text ?: codecFallback,
                codecError = notice?.takeIf { it.error }?.text ?: codecError)
        }
        is ScreenViewEvent.Stream -> copy(
            stream = event,
            firstCodec = firstCodec ?: event.codec,
            streams = (streams + (event.gop to event)).let { all ->
                // The decoder only looks up the groups of the pictures it holds.
                if (all.size <= STREAM_CACHE) all else all.entries.drop(all.size - STREAM_CACHE).associate { it.key to it.value }
            },
        )
        is ScreenViewEvent.Stats -> copy(
            stats = event.body,
            chainVirtualDisplay = ((event.body["chain"] as? JsonObject)?.get("virtualDisplay") as? JsonPrimitive)?.contentOrNull,
            chainRateCeilingKbps = ((event.body["chain"] as? JsonObject)?.get("rateCeilingKbps") as? JsonPrimitive)?.contentOrNull?.toLongOrNull(),
            delivery = DeliveryReport.of(event.body),
        )
        is ScreenViewEvent.RateCeiling -> copy(rateCeiling = event.state, rateAnswers = rateAnswers + 1)
        is ScreenViewEvent.ControlState -> copy(holder = event.holder, blocked = event.blocked, held = event.held)
        is ScreenViewEvent.Windows -> copy(windows = runCatching { screenJson.decodeFromJsonElement<ScreenWindows>(event.body) }.getOrNull())
        is ScreenViewEvent.Family -> copy(family = screenJson.decodeFromJsonElement<FamilyState>(event.body))
        is ScreenViewEvent.Error -> copy(errorCode = event.code, errorDetail = event.detail,
            codecError = ScreenCodecNotices.parse(event.detail, chosenCodec)?.takeIf { it.error }?.text ?: codecError)
        is ScreenViewEvent.Ended -> copy(ended = event, state = "ended",
            codecError = ScreenCodecNotices.parse(event.detail, chosenCodec)?.takeIf { it.error }?.text ?: codecError)
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

    /** The offers for the viewer's own choice „Codec“ and „Dekodierer“ (BK7b); a backend that does not know them offers [offers]. */
    fun offers(choice: ViewerCodecChoice): List<DecoderOffer> = offers()

    /** The pictures of session [session]. */
    fun pictures(session: Long): PictureSource

    suspend fun devices(roomId: String?): List<ScreenDevice>

    suspend fun probe(target: ScreenTarget): ScreenProbe

    /** The encoders of [target] (its probe); `null` when unknown, then a session opens with the stored choice. */
    suspend fun hostCodecs(target: ScreenTarget): List<HostCodec>? = null

    suspend fun open(
        target: ScreenTarget,
        mode: String,
        source: String?,
        displayWidth: Int,
        displayHeight: Int,
        maxFps: Int,
        decoders: List<DecoderOffer>,
    ): Long

    /** [fallback]: `lossless` is only the automatic fallback to the tiles, not the user's choice; the daemon sends
     *  it to the host but keeps the choice for a rebuilt session (round 11, N2). */
    suspend fun setMode(session: Long, mode: String, source: String? = null, width: Int? = null, height: Int? = null, fps: Int? = null, sharpness: Boolean = false, lossless: Boolean = false, fallback: Boolean = false)

    /** [open] with the viewer's choice „Codec“, „Dekodierer“ and „Höchste Datenrate“; a backend that does not know them opens without. */
    suspend fun openWith(target: ScreenTarget, mode: String, source: String?, displayWidth: Int, displayHeight: Int, maxFps: Int, decoders: List<DecoderOffer>, choice: ViewerCodecChoice): Long =
        open(target, mode, source, displayWidth, displayHeight, maxFps, decoders)

    /** „Datenrate“ changed during the session ([maxRate] in bit/s, 0 automatic); the mode and switches stay as given. */
    suspend fun setRateCeiling(session: Long, mode: String, sharpness: Boolean, lossless: Boolean, maxRate: Long, fallback: Boolean = false) = Unit

    /** „Übertragung“ ([delivery] `realtime` or `reliable`) changed during the session; `false` when the host does not take it. */
    suspend fun setDelivery(session: Long, delivery: String): Boolean = false

    /** „Neu messen“: the host measures the line again; `false` when the host does not take it. */
    suspend fun remeasure(session: Long): Boolean = false

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

    override fun offers(choice: ViewerCodecChoice) = ScreenDecoder.offers(choice)

    override fun pictures(session: Long): PictureSource = NativePictureSource(session)

    override suspend fun devices(roomId: String?) = ScreenApi.devices(roomId)

    override suspend fun probe(target: ScreenTarget) = ScreenApi.probe(target.relation, target.relationId, target.deviceId)

    override suspend fun hostCodecs(target: ScreenTarget) = ScreenCodecOffer.probe(target)

    override suspend fun open(target: ScreenTarget, mode: String, source: String?, displayWidth: Int, displayHeight: Int, maxFps: Int, decoders: List<DecoderOffer>) =
        ScreenApi.viewOpen(target.relation, target.relationId, target.deviceId, mode, source, decoders, displayWidth, displayHeight, maxFps, ScreenScaling.awaitRendererLimit(), target.expectedNodeId)

    override suspend fun setMode(session: Long, mode: String, source: String?, width: Int?, height: Int?, fps: Int?, sharpness: Boolean, lossless: Boolean, fallback: Boolean) =
        ScreenApi.viewSetMode(session, mode, source, width, height, fps, sharpness, lossless, fallback = fallback)

    override suspend fun openWith(target: ScreenTarget, mode: String, source: String?, displayWidth: Int, displayHeight: Int, maxFps: Int, decoders: List<DecoderOffer>, choice: ViewerCodecChoice) =
        ScreenApi.viewOpen(target.relation, target.relationId, target.deviceId, mode, source, decoders, displayWidth, displayHeight, maxFps, ScreenScaling.awaitRendererLimit(), target.expectedNodeId,
            // „Monitor“/„Auflösung“ only for a full view: a preview tile never makes a host attach or resize a display
            // (Gemini 3/20; the desktop's previews send none either, `screen_pool.rs`).
            maxRate = choice.maxRateBps, codec = choice.codec, decoder = choice.decoder,
            display = ScreenDisplayPrefs.current.takeIf { mode != "preview" }, delivery = choice.delivery)

    override suspend fun setRateCeiling(session: Long, mode: String, sharpness: Boolean, lossless: Boolean, maxRate: Long, fallback: Boolean) =
        ScreenApi.viewSetMode(session, mode, sharpness = sharpness, lossless = lossless, maxRate = maxRate, fallback = fallback)

    override suspend fun setDelivery(session: Long, delivery: String) = ScreenApi.viewSetDelivery(session, delivery)

    override suspend fun remeasure(session: Long) = ScreenApi.viewRemeasure(session)

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

internal data class ChoiceSnapshot(
    val choice: ViewerCodecChoice,
    val forceLossless: Boolean,
    val codecFallbackGop: Long?,
    val codecFallback: String?,
)

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
    /** The next visibility answer belongs to a report that kept the shown picture and input. */
    private var keepOnAck = false
    private var minFrame: Long? = null
    private var latestComplete: Long? = null
    /** A frame drawn before the host's `VISIBILITY_ACK` named the first frame input may follow; the ack decides on it. */
    private var earlyPresented: ScreenPresentedFrame? = null
    private var displayEpoch = 0L
    private var rejectedGop: Long? = null
    private var codecFallbackGop:Long?=null
    private var forceLossless=false
    private var bitmapPending=false
    /** Survives Surface disposal until this viewer session itself is released. */
    internal val decoderBuffers = ScreenPullBuffers(offered = { mutable.value.bufferBudget })
    internal val scaleWorker=ScreenScaleWorker {mutable.value.bufferBudget}
    private var wantedMode=ScreenModeRequest()
    private val commands=object:ScreenBackend by backend {
        override suspend fun setMode(session:Long,mode:String,source:String?,width:Int?,height:Int?,fps:Int?,sharpness:Boolean,lossless:Boolean,fallback:Boolean) {
            val forced=synchronized(lock) {forceLossless}
            // A `lossless` that only the forced fallback sets is marked as such (round 11, N2).
            backend.setMode(session,mode,source,width,height,fps,sharpness,lossless || forced,
                fallback = fallback || (forced && !lossless))
            synchronized(lock) {
                wantedMode=ScreenModeRequest(mode,sharpness,lossless)
            }
        }
    }
    private var lastPresented: ScreenPresentedFrame? = null
    private var closed = false
    private var inputJob: Job? = null

    /**
     * The choice „Codec“, „Dekodierer“ and „Höchste Datenrate“ this session runs with (BK7b): the one it was opened with,
     * or the one the host took within the session ([switchedTo], P1).
     */
    @Volatile var choice: ViewerCodecChoice = ViewerCodecChoice()
        private set

    /**
     * The source this session was opened with (`null` the primary monitor): the host shows it from the start, so a
     * `SET_MODE` names a source only when it differs (a named source makes the host open its capture again).
     */
    @Volatile var openedSource: String? = null
        private set

    /** The „Codec“ this session was opened with ([ViewerCodecChoice.reopenForVideo]). */
    @Volatile var openedCodec: String = ViewerCodecChoice.CODEC_AUTO
        private set

    /** The data-rate ceiling this session was opened with (`null` automatic); [switchedTo] never overwrites it. */
    @Volatile var openedMaxRateBps: Long? = null
        private set

    /**
     * What [stored] runs as with this session's host and this phone ([ScreenCodecOffer.sessionChoice]): the host's
     * encoders are asked for only when the choice depends on them; a choice this phone cannot decode runs as
     * „Automatisch“ without them. `null` from a backend without decoders (task tests): nothing is checked.
     */
    private suspend fun served(stored: ViewerCodecChoice): SessionCodec? {
        val offersFor = { wanted: ViewerCodecChoice -> backend.offers(wanted) }
        if (offersFor(stored).isEmpty()) return null
        val host = if (ScreenCodecOffer.hostMatters(stored, offersFor)) backend.hostCodecs(target) else null
        host?.let { found -> mutable.update { it.copy(hostCodecs = found) } }
        return ScreenCodecOffer.sessionChoice(stored, host, offersFor)
    }

    /**
     * Publishes [next] before `screen.viewSetDecoders` goes out so the decoder thread sees the new choice as soon as the
     * host's next picture group arrives, and clears a prior automatic lossless fallback (`host_pairing.rs` lets another
     * encoder try). Returns the previous state for [restoreChoice] when the host does not switch.
     */
    internal fun prepareDecoderSwitch(next: ViewerCodecChoice): ChoiceSnapshot = synchronized(lock) {
        val snapshot = ChoiceSnapshot(choice, forceLossless, codecFallbackGop, mutable.value.codecFallback)
        choice = next
        if (!next.lossless) {
            forceLossless = false
            codecFallbackGop = null
            mutable.value = mutable.value.copy(codecFallback = null)
        }
        snapshot
    }

    /**
     * The user chose the codec [codec] („Codec“, also the live switches „Verlustfrei“ and „Scharf (GPU)“ through
     * `SET_MODE`): an automatic lossless fallback of the codec before no longer overrules it, and the notices about the
     * codec before go (user rule of 2026-11-18: every choice is used exactly as made).
     */
    internal fun chooseCodec(codec: String) = synchronized(lock) {
        if (mutable.value.chosenCodec == codec) return@synchronized
        forceLossless = false
        codecFallbackGop = null
        mutable.value = mutable.value.copy(chosenCodec = codec, codecFallback = null, codecError = null)
    }

    internal fun restoreChoice(snapshot: ChoiceSnapshot) = synchronized(lock) {
        choice = snapshot.choice
        forceLossless = snapshot.forceLossless
        codecFallbackGop = snapshot.codecFallbackGop
        mutable.value = mutable.value.copy(codecFallback = snapshot.codecFallback)
    }

    /** The host took [next] within this session (`screen.viewSetDecoders` answered `switched`): the decoder thread reads it for the next picture group. */
    internal fun switchedTo(next: ViewerCodecChoice) {
        prepareDecoderSwitch(next)
    }

    /** The core's session number; 0 before [open] returned. */
    @Volatile var id: Long = 0
        private set

    val state: StateFlow<ScreenSessionState> = mutable.asStateFlow()

    val deviceTarget: ScreenTarget get() = target

    /** Opens the session; a failure arrives as [CoreException] and leaves the session closed. */
    suspend fun open(mode: String, source: String?, displayWidth: Int, displayHeight: Int, maxFps: Int, decoders: List<DecoderOffer>, stored: ViewerCodecChoice = ViewerCodecChoice()) {
        // User rule of 2026-11-24: a stored choice the host cannot serve runs as „Automatisch“ for this session only (a
        // preview offers the tiles alone and keeps its own rule, `previewRefusal`).
        val served = if (mode == "preview") null else served(stored)
        val choice = served?.choice ?: stored
        val offers = if (choice == stored) decoders else backend.offers(choice.copy(replaces = null))
        this.choice = choice
        this.openedSource = source
        this.openedCodec = choice.codec
        this.openedMaxRateBps = choice.maxRateBps
        synchronized(lock) {
            // The host opens with the choice („Scharf (GPU)“, „Verlustfrei“, `set_codec_preference`): a later `SET_MODE`
            // that names only another rate or source keeps it, so the host keeps its encoder (round 12, B3, as the
            // desktop's `preview_set_mode`); before, a preview's interval or the way back from the full view cleared it.
            wantedMode=ScreenModeRequest(if(mode=="preview") "preview" else "full", sharpness = choice.sharp, lossless = choice.lossless)
            mutable.value = mutable.value.copy(chosenCodec = choice.codec, sessionNotes = served?.notes.orEmpty())
        }
        val backlog = ArrayList<ScreenViewEvent>()
        collector = scope.launch(start = CoroutineStart.UNDISPATCHED) {
            backend.events.collect { event ->
                synchronized(lock) {
                    if (id == 0L) backlog.add(event) else if (event.session == id) applyEvent(event)
                }
            }
        }
        // The opening runs to the core's answer even when the caller leaves meanwhile (a tile that left the screen): the
        // core's call cannot be taken back, so a session it opened is closed here instead of running without an owner,
        // and the collector, which keeps every event while no id is known, ends on every way out (Codex 12/6).
        // The id is kept inside the block: a cancellation reported only on the way out of `withContext` still finds it.
        var answer: Long? = null
        val failed = try {
            withContext(NonCancellable) {
                answer = backend.openWith(target, mode, source, displayWidth, displayHeight, maxFps, offers, choice)
            }
            null
        } catch (e: Throwable) {
            e
        }
        val opened = answer ?: run {
            collector?.cancel()
            scope.cancel()
            throw failed ?: CoreException("internal", "Die Sitzung hat keine Nummer erhalten.")
        }
        if (failed != null || !currentCoroutineContext().isActive || synchronized(lock) { closed }) {
            collector?.cancel()
            withContext(NonCancellable) {
                try { backend.close(opened) } catch (e: CoreException) { Unit }
            }
            scope.cancel()
            if (failed != null) throw failed
            currentCoroutineContext().ensureActive()
            throw CoreException("closed", "Die Sitzung wurde während des Öffnens geschlossen.")
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
        // A second `accepted`: the daemon rebuilt the host session without a new one here (Gemini 3/9.4). Its frame
        // ids count from 1 again (the bridge keeps the group ids rising, `screen_view_epoch.rs`): a new picture epoch,
        // no receipt of the connection before counts, and the visibility goes to the new host session again.
        if (event is ScreenViewEvent.Accepted && old.accepted) {
            displayEpoch++
            minFrame = null
            latestComplete = null
            earlyPresented = null
            lastPresented = null
            rejectedGop = null
            keepOnAck = false
            visibilitySentModern = false
            // The automatic fallback to the tiles and the first codec belonged to the host session before: the new one
            // negotiates afresh (Codex 9/8). The user's own choice (`chosenCodec`, `choice`) stays.
            forceLossless = false
            codecFallbackGop = null
            // The daemon replays the user's own choice into the rebuilt host session: every `SET_MODE` whose
            // `lossless` only the fallback set left marked as such (`fallback`), and `ModeMemory` keeps the choice
            // (round 11, N2). The new session therefore opens as chosen; nothing has to follow it.
            mutable.value = old.copy(inputReady = false, ackedMinFrame = null, held = null, blocked = null,
                firstCodec = null, codecFallback = null)
        }
        // The host fell back to the tiles: the viewer keeps them only where the choice allows it („Automatisch“,
        // „Verlustfrei“); under a named codec the fallback is an error ([ScreenCodecNotices]) and changes nothing here.
        if (event is ScreenViewEvent.State &&
            ScreenCodecNotices.parse(event.reason, old.chosenCodec)?.let { it.running == "tiles" && !it.error } == true) forceLossless = true
        val familyLayoutChanged = event is ScreenViewEvent.Family &&
            screenJson.decodeFromJsonElement<FamilyState>(event.body).snapshot?.layout_revision != old.family?.snapshot?.layout_revision
        // A new picture group of the same codec, size and family layout is a key frame of the same picture (the
        // host keeps input across it, ae1da484): the presentation receipts of the old group go, the display epoch
        // and the input readiness stay (Codex 3/13). Another codec, size or layout is a new reference.
        val newGop = event is ScreenViewEvent.Stream && old.stream?.gop != event.gop
        val sameReference = newGop && event is ScreenViewEvent.Stream && old.stream?.let {
            it.codec == event.codec && it.width == event.width && it.height == event.height &&
                it.familyLayoutRevision == event.familyLayoutRevision
        } == true
        if (sameReference) lastPresented = null
        if (newGop && !sameReference ||
            event is ScreenViewEvent.State && event.state != old.state && event.state != "streaming" ||
            event is ScreenViewEvent.Ended || familyLayoutChanged) {
            displayEpoch++
            latestComplete = null
            earlyPresented = null
            lastPresented = null
            mutable.value = old.copy(inputReady = false)
        }
        mutable.update { it.apply(event) }
        if (event is ScreenViewEvent.VisibilityAck && event.geometryGeneration == visibilityReport?.geometryGeneration &&
            keepOnAck) {
            // The answer to a report that kept the picture: floor and input stay (`publishVisibility`).
            keepOnAck = false
        } else if (event is ScreenViewEvent.VisibilityAck && event.geometryGeneration == visibilityReport?.geometryGeneration) {
            minFrame = event.minFrameId
            // A frame drawn before this ack arrived counts once the ack names it (a static screen sends no other one).
            val early = earlyPresented
            earlyPresented = null
            if (early != null && displayable(early, beforeAck = false)) acceptPresented(early)
            mutable.value = mutable.value.copy(ackedMinFrame = event.minFrameId, visibilityAcks = mutable.value.visibilityAcks + 1,
                inputReady = visibilityReport?.state != "hidden" &&
                latestComplete?.let { it >= event.minFrameId } == true)
        }
        val report = visibilityReport
        if (mutable.value.frameContract && !visibilitySentModern && report != null) {
            visibilitySentModern = true
            minFrame = null
            latestComplete = null
            earlyPresented = null
            mutable.value = mutable.value.copy(inputReady = false)
            inputs.trySend(ScreenSessionCommand.Visibility(report))
        }
    }

    /** Release events are queued before this report by the view on hiding. */
    internal fun publishVisibility(facts: ScreenVisibilityFacts) = synchronized(lock) {
        if (visibilityFacts == facts) return@synchronized
        val before = visibilityFacts
        visibilityFacts = facts
        val previous = visibilityReport
        val report = facts.report((previous?.geometryGeneration ?: 0L)+1L)
        val state = report.state
        visibilityReport = report
        // A view that stays visible on the same source changed only its visible area (a pan, a zoom): the host keeps
        // encoder and held keys for it, so the shown picture, its receipts and the input stay here too, as on the
        // desktop (`ResumeGate::renumber`); only the new rectangles go out (Codex 8/6). An acknowledged floor stays.
        if (previous?.state == "visible" && state == "visible" && before?.sourceKey == facts.sourceKey) {
            keepOnAck = minFrame != null
            if (mutable.value.frameContract) {
                visibilitySentModern = true
                inputs.trySend(ScreenSessionCommand.Visibility(report))
            }
            return@synchronized
        }
        keepOnAck = false
        displayEpoch++
        minFrame = null
        latestComplete = null
        earlyPresented = null
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
        earlyPresented = null
        lastPresented = null
        mutable.value = mutable.value.copy(inputReady = false)
    }

    fun presentationEpoch(): Long = synchronized(lock) { displayEpoch }
    /** Shared by decoder/Surface handovers: one immutable snapshot may wait on Main. */
    fun beginBitmapSnapshot():Boolean = synchronized(lock) {
        if(closed || bitmapPending) false else {bitmapPending=true;true}
    }
    fun endBitmapSnapshot() = synchronized(lock) {bitmapPending=false}

    fun isDisplayable(frame: ScreenPresentedFrame): Boolean = synchronized(lock) { displayable(frame, beforeAck = false) }

    /**
     * [frame] belongs to the shown picture but was drawn before the host's `VISIBILITY_ACK` named the first frame input
     * may follow: [confirmPresented] keeps it, and the ack counts it when it names it (a static screen sends no newer one).
     */
    fun awaitsAck(frame: ScreenPresentedFrame): Boolean = synchronized(lock) {
        mutable.value.frameContract && displayable(frame, beforeAck = true)
    }

    /** [isDisplayable]; [beforeAck]: the same, while the host's `VISIBILITY_ACK` has not named its first frame yet. */
    private fun displayable(frame: ScreenPresentedFrame, beforeAck: Boolean): Boolean {
        val current = mutable.value
        return !closed && frame.session == id && frame.epoch == displayEpoch && current.ended == null &&
            current.accepted && current.state in listOf("streaming", "starting") &&
            frame.gopId == current.stream?.gop && frame.gopId != rejectedGop && visibilityReport?.state != "hidden" &&
            (!forceLossless || current.stream?.codec == "tiles") &&
            (!current.frameContract || (if (beforeAck) minFrame == null else minFrame?.let { frame.frameId >= it } == true))
            && (!familyRequired || current.family?.snapshot?.layout_revision?.let {
                current.stream?.familyLayoutRevision == it
            } == true)
    }

    /** Only renderer callbacks invoke this; queued decoder and scaler output cannot open input. */
    fun confirmPresented(frame: ScreenPresentedFrame) = synchronized(lock) {
        if (!displayable(frame, beforeAck = false)) {
            if (mutable.value.frameContract && displayable(frame, beforeAck = true) &&
                earlyPresented?.let { it.frameId >= frame.frameId } != true) earlyPresented = frame
            return@synchronized
        }
        acceptPresented(frame)
    }

    /** [frame] was drawn and may open input; the caller holds [lock]. */
    private fun acceptPresented(frame: ScreenPresentedFrame) {
        latestComplete = maxOf(latestComplete ?: 0L, frame.frameId)
        if (mutable.value.frameContract && (lastPresented?.let {
            it.epoch == frame.epoch && it.gopId == frame.gopId && it.frameId >= frame.frameId
        } != true)) {
            lastPresented = frame
            if (inputs.trySend(ScreenSessionCommand.Presented(frame)).isFailure) return
        }
        mutable.value = mutable.value.copy(inputReady = true)
    }

    /** A decoder failure remains valid when queue recovery has already announced a newer GOP. */
    fun hybridUnavailable(gopId: Long, reason: String, mode: String = "full") = synchronized(lock) {
        val current=mutable.value
        if(closed || id==0L || current.ended!=null || forceLossless ||
            !current.streams.containsKey(gopId) || codecFallbackGop==gopId) return@synchronized
        if(mode!="full" && mode!="preview") return@synchronized
        // A named codec never gives way to the tiles on its own, and neither does „Dekodierer: Hardware“: the tiles
        // are drawn in software (Codex 5/13, Gemini 5/7). „Verlustfrei“ with „Hardware“ are two choices that cannot
        // both hold: said as such, as on the desktop. The reason is shown, the choice stays.
        val hardwareChosen = choice.decoder == ViewerCodecChoice.DECODER_HARDWARE
        if (!ViewerCodecChoice.tilesFallbackAllowed(current.chosenCodec) || hardwareChosen) {
            codecFallbackGop = gopId
            val text = if (current.chosenCodec == ViewerCodecChoice.CODEC_LOSSLESS && hardwareChosen)
                ScreenCodecNotices.losslessNeedsSoftware()
            else ScreenCodecNotices.decoderUnavailable(current.chosenCodec, choice.decoder, reason)
            mutable.value = current.copy(codecError = text)
            return@synchronized
        }
        codecFallbackGop=gopId;forceLossless=true;rejectedGop=current.stream?.gop;displayEpoch++
        latestComplete=null;earlyPresented=null;lastPresented=null
        mutable.value=mutable.value.copy(inputReady=false,
            codecFallback="Video-Einzelinputausgabe nicht bestätigt. Verlustfreie Übertragung angefordert. ${reason.take(768)}")
        inputs.trySend(ScreenSessionCommand.Call {session->
            val wanted=synchronized(lock) {wantedMode}
            // The host owns the separate full/preview size and rate choices.
            // A codec fallback changes none of those choices.
            backend.setMode(session,wanted.mode,source=null,width=null,height=null,
                fps=null,sharpness=wanted.sharpness,lossless=true,fallback=!wanted.lossless)
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

    /**
     * „Höchste Datenrate“ ([bps] bit/s, `null` automatic) changes while the session runs. The core sends it as `RATE_CEILING`
     * to a host that announced it; the wanted mode and switches go out unchanged beside it.
     */
    fun setMaxRate(bps: Long?) {
        if (id == 0L) return
        // Every other writer of the state holds the lock while it reads and writes; a count outside it could be overwritten.
        synchronized(lock) { mutable.value = mutable.value.copy(rateSends = mutable.value.rateSends + 1) }
        call { session ->
            val wanted = synchronized(lock) {wantedMode}
            val forced = synchronized(lock) {forceLossless}
            try {
                setRateCeiling(session, wanted.mode, wanted.sharpness, wanted.lossless || forced, bps ?: 0L,
                    fallback = forced && !wanted.lossless)
            } catch (e: CoreException) {
                // No frame left the core, so no `rateCeiling` answer follows: this send does not count.
                synchronized(lock) { mutable.value = mutable.value.copy(rateSends = (mutable.value.rateSends - 1).coerceAtLeast(0)) }
                throw e
            }
        }
    }

    /**
     * „Übertragung“ ([delivery] `realtime` or `reliable`) changes while the session runs: the core sends it to a host whose
     * statistics name „Übertragung“ (`screen.viewSetDelivery`); an older host keeps its way, the choice applies from the
     * next session on. A session that is not open yet opens with the stored choice.
     */
    fun setDelivery(delivery: String) {
        if (id == 0L) return
        synchronized(lock) { choice = choice.copy(delivery = delivery) }
        call { session -> setDelivery(session, delivery) }
    }

    /** „Neu messen“: the host measures the line again; its statistics name the new rate ([ScreenSessionState.delivery]). */
    fun remeasure() {
        call { session -> remeasure(session) }
    }

    /** Runs a call on this session in the background; its failure becomes the shown error. */
    fun call(block: suspend ScreenBackend.(Long) -> Unit) {
        val session = id
        if (session == 0L) return
        inputs.trySend(ScreenSessionCommand.Call(block))
    }

    /** Closes the session in the core once the queued input has gone out; closing twice is fine. */
    fun close() {
        synchronized(lock) { closed = true; displayEpoch++; latestComplete = null; earlyPresented = null; mutable.value = mutable.value.copy(inputReady = false) }
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
