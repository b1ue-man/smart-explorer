package app.smartexplorer.android.screen

import android.app.KeyguardManager
import android.content.Context
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.projection.MediaProjection
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.WindowManager
import app.smartexplorer.android.api.ScreenApi
import app.smartexplorer.android.api.ScreenHostSession
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.NativeBridge
import app.smartexplorer.android.core.ScreenHostStart
import app.smartexplorer.android.core.ScreenSessionChanged
import app.smartexplorer.android.service.AccessApprovalNotifier
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull

/**
 * The app side of this phone as screen host (plan 2026-10-17, SCR-M8): reports what the phone can
 * (`screen.hostPrereqs`), takes the sessions the core starts (`screenHostStart`) and runs each one:
 * the test picture right away, still previews through the accessibility service, every other source
 * only after the system dialog („Freigeben“ → [ScreenConsentActivity] → [ScreenHostService]). One
 * thread per session follows the core's control messages (`configure`, `keyframe`, `bitrate`,
 * `still`, `consent`, `stop`); one thread hands the controlling viewer's input to
 * [ScreenControlService]. Logs name sessions and operations only, never screen content.
 */
internal object ScreenHostControl {
    private const val TAG = "SmartExplorerScreen"
    private const val CONTROL_POLL_MS = 1_000

    /** A consent activity started from the device's question claims the session in this time. */
    private const val CLAIM_GRACE_MS = 700L
    private const val STROKE_GRACE_MS = 500L

    private class HostSession(val start: ScreenHostStart) {
        val lock = Any()

        @Volatile
        var ended = false

        @Volatile
        var config: EncoderConfig? = null
        var encoder: ScreenEncoder? = null
        var projection: MediaProjection? = null
        var display: VirtualDisplay? = null
        var synthetic: SyntheticSurfaceSource? = null
        val still = ScreenStillSource { buffer, length -> NativeBridge.screenPush(start.session, buffer, 0, length, 0, FLAG_STILL) }
    }

    /**
     * The session id of an in-process capture probe (device tests only): its projection still comes
     * from the system dialog, and [probe] takes it instead of a viewer's session.
     */
    const val PROBE_SESSION = -1L

    @Volatile
    var probe: ((MediaProjection) -> Unit)? = null

    @Volatile
    private var probeProjection: MediaProjection? = null

    private val started = AtomicBoolean(false)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val sessions = ConcurrentHashMap<Long, HostSession>()
    private val pendingConsent = ConcurrentHashMap<Long, ScreenHostStart>()
    private val claimed: MutableSet<Long> = ConcurrentHashMap.newKeySet()
    private lateinit var app: Context
    private var input: Thread? = null

    fun start(context: Context) {
        if (!started.compareAndSet(false, true)) return
        app = context.applicationContext
        scope.launch {
            Core.events.collect { event ->
                when (event) {
                    is ScreenHostStart -> onStart(event)
                    is ScreenSessionChanged -> if (event.state == "ended") sessions[event.session]?.let { teardown(it) }
                    else -> Unit
                }
            }
        }
        scope.launch { reportPrereqs() }
    }

    /** The accessibility service went on or off: the core learns it at once. */
    fun accessibilityChanged(context: Context) {
        if (!started.get()) start(context)
        scope.launch { reportPrereqs() }
    }

    suspend fun reportPrereqs() {
        try {
            ScreenApi.prereqs(
                accessibility = ScreenControlService.state(app),
                usageAccess = RunningApps.usageAccess(app),
                // App windows in the system dialog: Android 14 QPR2; API 35 is the safe floor.
                appWindowCapture = Build.VERSION.SDK_INT >= Build.VERSION_CODES.VANILLA_ICE_CREAM,
                display = displaySize(),
                encoders = ScreenEncoder.offers(),
                apps = RunningApps.list(app),
            )
        } catch (e: CoreException) {
            Log.w(TAG, "screen.hostPrereqs: ${e.kind}")
        }
    }

    private fun displaySize(): Pair<Int, Int> {
        val bounds = app.getSystemService(WindowManager::class.java).maximumWindowMetrics.bounds
        return bounds.width() to bounds.height()
    }

    /** The consent activity of a question takes [session]; `false` when someone already did. */
    fun claim(session: Long): Boolean = claimed.add(session)

    /** A waiting full session of [deviceId] that no one claimed yet. */
    fun claimPending(deviceId: String): ScreenHostStart? =
        pendingConsent.values.firstOrNull { it.viewerDeviceId == deviceId && !it.still && claimed.add(it.session) }

    private suspend fun onStart(start: ScreenHostStart) {
        val session = HostSession(start)
        sessions[start.session] = session
        thread(name = "screen-control-${start.session}", isDaemon = true) { controlLoop(session) }
        ensureInput()
        try {
            when {
                start.source == "synthetic" -> ScreenApi.ready(start.session, start.maxWidth, start.maxHeight, "synthetic", false, false)
                start.still -> {
                    if (ScreenControlService.current() == null) {
                        ScreenApi.ended(start.session, "accessibility_off")
                    } else {
                        val (width, height) = displaySize()
                        ScreenApi.ready(start.session, width, height, "tiles", false, false)
                    }
                }
                else -> requestConsent(start)
            }
        } catch (e: CoreException) {
            Log.w(TAG, "Bildschirmsitzung ${start.session}: ${e.kind}")
        }
        refreshNotification()
    }

    /** Asks the host user for the system dialog unless a consent activity claims the session. */
    private suspend fun requestConsent(start: ScreenHostStart) {
        pendingConsent[start.session] = start
        delay(CLAIM_GRACE_MS)
        if (!claim(start.session)) return
        val activity = AccessApprovalNotifier.foregroundActivity()
        if (activity != null) {
            try {
                activity.startActivity(ScreenConsentActivity.sessionIntent(activity, start))
                return
            } catch (e: RuntimeException) {
                Log.w(TAG, "Zustimmung nicht geöffnet: ${e.message}")
            }
        }
        if (!ScreenHostNotifier.showConsent(app, start)) ScreenApi.ended(start.session, "approval_unavailable")
    }

    private fun controlLoop(session: HostSession) {
        val id = session.start.session
        while (!session.ended) {
            val raw = try {
                NativeBridge.screenControl(id, CONTROL_POLL_MS)
            } catch (e: LinkageError) {
                Log.e(TAG, "screenControl fehlt in der nativen Bibliothek", e)
                return
            } ?: continue
            val message = try {
                Core.json.parseToJsonElement(raw) as? JsonObject
            } catch (e: IllegalArgumentException) {
                null
            } ?: continue
            when (message.text("op")) {
                "configure" -> {
                    session.config = EncoderConfig(
                        epoch = message.int("epoch"),
                        width = message.int("width"),
                        height = message.int("height"),
                        fps = message.int("fps").coerceAtLeast(1),
                        bitrate = message.long("bitrate").coerceAtMost(Int.MAX_VALUE.toLong()).toInt(),
                        profile = message.text("profile") ?: "high",
                    )
                    applyConfig(session)
                }
                "keyframe" -> synchronized(session.lock) { session.encoder?.requestKeyFrame() }
                "bitrate" -> synchronized(session.lock) {
                    session.encoder?.setBitrate(message.long("bps").coerceAtMost(Int.MAX_VALUE.toLong()).toInt())
                }
                "still" -> ScreenControlService.current()?.let { service -> session.still.capture(service) {} }
                "consent" -> scope.launch { requestConsent(session.start) }
                "stop" -> teardown(session)
            }
        }
    }

    /** (Re)starts the encoder of [session] for its configuration once its source runs. */
    private fun applyConfig(session: HostSession) = synchronized(session.lock) {
        val config = session.config ?: return@synchronized
        val synthetic = session.start.source == "synthetic"
        val projection = session.projection
        if (session.ended || (!synthetic && projection == null)) return@synchronized
        val epoch = config.epoch
        val encoder = try {
            ScreenEncoder(config) { buffer, offset, length, pts, key ->
                val flags = epochFlags(epoch) or (if (key) FLAG_KEYFRAME else 0)
                NativeBridge.screenPush(session.start.session, buffer, offset, length, pts, flags)
            }
        } catch (e: RuntimeException) {
            Log.w(TAG, "Kodierer nicht gestartet: ${e.message}")
            scope.launch { endQuietly(session.start.session, "capture_failed") }
            return@synchronized
        }
        session.synthetic?.stop()
        val old = session.encoder
        if (synthetic) {
            session.synthetic = SyntheticSurfaceSource(encoder.surface, config.width, config.height, config.fps)
        } else if (projection != null) {
            val dpi = app.resources.displayMetrics.densityDpi
            val display = session.display
            if (display == null) {
                session.display = projection.createVirtualDisplay(
                    "se-screen-${session.start.session}",
                    config.width,
                    config.height,
                    dpi,
                    DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,
                    encoder.surface,
                    null,
                    null,
                )
            } else {
                display.resize(config.width, config.height, dpi)
                display.surface = encoder.surface
            }
        }
        session.encoder = encoder
        old?.release()
    }

    /** [ScreenHostService] holds the system dialog's projection for [session]. */
    fun attachProjection(context: Context, session: Long, projection: MediaProjection, gone: () -> Unit) {
        val taker = probe
        if (session == PROBE_SESSION && taker != null) {
            probeProjection = projection
            projection.registerCallback(
                object : MediaProjection.Callback() {
                    override fun onStop() {
                        probeProjection = null
                        gone()
                    }
                },
                Handler(Looper.getMainLooper()),
            )
            taker(projection)
            return
        }
        val host = sessions[session]
        if (host == null || host.ended) {
            projection.stop()
            gone()
            return
        }
        projection.registerCallback(
            object : MediaProjection.Callback() {
                override fun onStop() {
                    if (!host.ended) {
                        val locked = context.getSystemService(KeyguardManager::class.java)?.isKeyguardLocked == true
                        scope.launch { endQuietly(session, if (locked) "host_locked" else "host_stopped") }
                        teardown(host)
                    }
                    gone()
                }
            },
            Handler(Looper.getMainLooper()),
        )
        synchronized(host.lock) { host.projection = projection }
        pendingConsent.remove(session)
        val (width, height) = displaySize()
        val (name, hardware) = ScreenEncoder.best() ?: ("mediacodec" to false)
        scope.launch {
            try {
                ScreenApi.ready(session, width, height, name, hardware, projection = true)
            } catch (e: CoreException) {
                teardown(host)
            }
        }
        applyConfig(host)
        refreshNotification()
    }

    suspend fun projectionFailed(session: Long, reason: String) {
        endQuietly(session, reason)
        sessions[session]?.let { teardown(it) }
    }

    fun projectionCount(): Int = sessions.values.count { it.projection != null } + (if (probeProjection != null) 1 else 0)

    fun releaseProjections() = sessions.values.filter { it.projection != null }.forEach { teardown(it) }

    /** „Beenden“ of the host user: every session ends. */
    suspend fun stopAll(context: Context) {
        try {
            ScreenApi.stop(null)
        } catch (e: CoreException) {
            Log.w(TAG, "screen.hostStop: ${e.kind}")
        }
        sessions.values.forEach { teardown(it) }
        ScreenHostNotifier.showSessions(context, emptyList())
    }

    fun sessionsView(): List<ScreenHostSession> = sessions.values.map {
        ScreenHostSession(
            session = it.start.session,
            source = it.start.source,
            still = it.start.still,
            viewerName = it.start.viewerName,
            viewerDeviceId = it.start.viewerDeviceId,
            projection = it.projection != null,
        )
    }

    private fun teardown(session: HostSession) {
        synchronized(session.lock) {
            if (session.ended) return
            session.ended = true
            session.synthetic?.stop()
            session.display?.release()
            session.encoder?.release()
            session.projection?.stop()
            session.synthetic = null
            session.display = null
            session.encoder = null
            session.projection = null
        }
        val id = session.start.session
        sessions.remove(id)
        pendingConsent.remove(id)
        claimed.remove(id)
        ScreenHostNotifier.cancelConsent(app, id)
        refreshNotification()
    }

    private suspend fun endQuietly(session: Long, reason: String) {
        try {
            ScreenApi.ended(session, reason)
        } catch (e: CoreException) {
            // `not_found`: already ended.
        }
    }

    /** The ongoing notification of the sessions without MediaProjection (the service shows its own). */
    private fun refreshNotification() {
        ScreenHostNotifier.showSessions(app, sessionsView().filter { !it.projection && it.source != "synthetic" })
    }

    /** One thread hands the controlling viewer's input to the accessibility service, in order. */
    @Synchronized
    private fun ensureInput() {
        if (input?.isAlive == true) return
        input = thread(name = "screen-input", isDaemon = true) {
            val mapper = GestureMapper({ displaySize().first }, { displaySize().second })
            while (sessions.isNotEmpty()) {
                val raw = try {
                    NativeBridge.screenControl(0, CONTROL_POLL_MS)
                } catch (e: LinkageError) {
                    return@thread
                } ?: continue
                val message = try {
                    Core.json.parseToJsonElement(raw) as? JsonObject
                } catch (e: IllegalArgumentException) {
                    null
                } ?: continue
                val events = message["events"] as? JsonArray ?: continue
                val service = ScreenControlService.current() ?: continue
                for (action in mapper.map(events)) {
                    val done = CountDownLatch(1)
                    GestureMapper.perform(service, action) { done.countDown() }
                    val wait = (action as? PhoneAction.Stroke)?.durationMs ?: 0
                    done.await(wait + STROKE_GRACE_MS, TimeUnit.MILLISECONDS)
                }
            }
        }
    }

    private fun JsonObject.text(key: String): String? = (this[key] as? JsonPrimitive)?.contentOrNull

    private fun JsonObject.int(key: String): Int = (this[key] as? JsonPrimitive)?.intOrNull ?: 0

    private fun JsonObject.long(key: String): Long = (this[key] as? JsonPrimitive)?.longOrNull ?: 0
}
