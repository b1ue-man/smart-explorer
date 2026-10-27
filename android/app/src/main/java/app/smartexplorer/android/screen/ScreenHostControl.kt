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
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
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
 *
 * The keyguard does not end a session here: Android decides. On Android 15 QPR1+ the system stops
 * a running MediaProjection when the screen locks (`onStop`, reported as `host_locked`) unless the
 * app is exempt (AOSP `MediaProjectionStopController.isExempt`, e.g. the `PROJECT_MEDIA` app op
 * set to allow); an exempt projection keeps capturing the lock screen, where the bouncer window is
 * `FLAG_SECURE` and therefore black. Accessibility stills (`takeScreenshot`) and gestures
 * (`dispatchGesture`) have no keyguard check in AOSP, so previews and input reach the lock screen.
 */
internal object ScreenHostControl {
    private const val TAG = "SmartExplorerScreen"
    private const val CONTROL_POLL_MS = 1_000

    /** A consent activity started from the device's question claims the session in this time. */
    private const val CLAIM_GRACE_MS = 700L
    private const val STROKE_GRACE_MS = 500L

    private class HostSession(val start: ScreenHostStart) {
        val lock = Any()
        val readyLock = Mutex()

        @Volatile
        var ended = false
        @Volatile var captureActive=true

        @Volatile
        var config: EncoderConfig? = null
        var encoder: ScreenEncoder? = null
        @Volatile var encoderToken:Any?=null
        @Volatile var encoderReadyToken:Any?=null
        var pendingCapture:Long?=null
        var frames: ScreenProjectionFrames? = null
        @Volatile var projection: MediaProjection? = null
        @Volatile var identity: ScreenProjectionIdentity? = null
        var display: VirtualDisplay? = null
        var synthetic: SyntheticSurfaceSource? = null
        var encoderRetirements=0
        var pendingConfigStart=false
        val still = ScreenStillSource(start.session,{displaySize()}) { buffer, length, generation ->
            if (ended || !captureActive || projection != null) -1
            else NativeBridge.screenPushAdmitted(start.session,generation,buffer,0,length,0,FLAG_STILL)
        }
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

    fun requestedSource(session: Long): String? = sessions[session]?.start?.source

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
                ScreenProjectionIdentity.requiresPackageProof(start.source) -> requestConsent(start)
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
                "configure" -> synchronized(session.lock) {
                    try {
                        session.pendingCapture = null
                        session.config = EncoderConfig(
                            epoch = message.int("epoch"),
                            width = message.int("width"),
                            height = message.int("height"),
                            fps = message.int("fps").coerceAtLeast(1),
                            bitrate = message.long("bitrate").coerceAtMost(Int.MAX_VALUE.toLong()).toInt(),
                            profile = message.text("profile") ?: "high",
                            hybrid = message["hybrid"] == JsonPrimitive(true),
                            codec = message.text("codec") ?: "h264",
                        )
                        applyConfig(session)
                    } catch (error: RuntimeException) {
                        encoderFailed(
                            session, session.config?.epoch ?: 0,
                            error.message ?: "Capturekonfiguration abgelehnt",
                        )
                    }
                }
                "capture_active" -> try { synchronized(session.lock) {
                    val active=message["active"]==JsonPrimitive(true)
                    if(session.ended) return@synchronized
                    session.captureActive=active;session.still.setCaptureActive(active)
                    if(!active) {
                        session.pendingConfigStart=false
                        session.pendingCapture=null
                        session.display?.surface=null
                        check(session.frames?.close()!=false) {"Capturethread konnte nicht pausiert werden"};session.frames=null
                        val source=session.synthetic;val encoder=session.encoder
                        if(source!=null) {
                            session.synthetic=null;session.encoder=null
                            retireEncoder(session,source,encoder)
                        } else {encoder?.release();session.encoder=null}
                    }
                    val packet=java.nio.ByteBuffer.allocateDirect(9).order(java.nio.ByteOrder.LITTLE_ENDIAN)
                    packet.put(if(active) 1.toByte() else 0.toByte());packet.putLong(message.long("request"))
                    NativeBridge.screenPush(id,packet,0,9,0,0x20)
                }} catch(e: RuntimeException) {encoderFailed(session,session.config?.epoch ?: 0,"Capturezustand: ${e.message}")}
                "capture" -> synchronized(session.lock) {
                    val frame=message.long("frameId")
                    if(frame>0 && session.captureActive && !session.ended) {
                        val token=session.encoderToken
                        if(session.config?.hybrid==true && (token==null || session.encoderReadyToken!==token || session.frames==null))
                            session.pendingCapture=frame
                        else session.frames?.capture(frame)
                    }
                }
                "keyframe" -> synchronized(session.lock) { session.encoder?.requestKeyFrame() }
                "bitrate" -> synchronized(session.lock) {
                    session.encoder?.setBitrate(message.long("bps").coerceAtMost(Int.MAX_VALUE.toLong()).toInt())
                }
                "still" -> {
                    val projected=synchronized(session.lock) {
                        val config=session.config
                        val identity=session.identity
                        if(config==null) null else session.frames?.also { frames -> frames.captureStill(message.long("frameId")) { pixels ->
                            captureCurrent(session,identity,config.epoch) && pushStill(session,config,pixels)
                        } }
                    }
                    // An app chosen in MediaProjection must never fall back to a full-display screenshot.
                    if(projected==null && session.start.still && session.projection==null && !session.ended)
                        ScreenControlService.current()?.let { service -> session.still.capture(service) {} }
                }
                "consent" -> scope.launch { requestConsent(session.start) }
                "stop" -> teardown(session)
            }
        }
    }

    /** (Re)starts the encoder of [session] for its configuration once its source runs. */
    private fun applyConfig(session: HostSession): Unit = synchronized(session.lock) {
        val config = session.config ?: return@synchronized
        val synthetic = session.start.source == "synthetic"
        val projection = session.projection
        val identity = session.identity
        if (session.ended || !session.captureActive || (!synthetic && projection == null)) return@synchronized
        if (!synthetic && (projection == null || identity?.matches(projection) != true)) return@synchronized
        if(session.encoderRetirements!=0) {session.pendingConfigStart=true;return@synchronized}
        // This epoch replaces the old output. End its actual producer before
        // asking Rust to fund the replacement; retained asynchronous owners
        // remain charged until the existing retirement callback completes.
        if(session.encoder!=null || session.frames!=null || session.synthetic!=null) {
            session.encoderToken=null;session.encoderReadyToken=null;session.pendingCapture=null
            session.display?.surface=null
            check(session.frames?.close()!=false) {"Alter Capturethread ist noch nicht beendet"}
            session.frames=null
            val oldSource=session.synthetic;val oldEncoder=session.encoder
            session.synthetic=null;session.encoder=null
            if(oldSource!=null) {
                session.pendingConfigStart=true
                retireEncoder(session,oldSource,oldEncoder)
                return@synchronized
            }
            oldEncoder?.release()
        }
        session.pendingConfigStart=false
        val epoch = config.epoch
        val budget=ScreenCaptureBudget.open(session.start.session,config.width,config.height,when {
            config.codec=="tiles" -> ScreenCaptureBudget.PROJECTION_TILES
            synthetic -> ScreenCaptureBudget.SYNTHETIC_SURFACE
            config.hybrid -> ScreenCaptureBudget.PROJECTION_HYBRID
            else -> ScreenCaptureBudget.PROJECTION_SURFACE
        })
        if(config.codec=="tiles" && projection!=null) {
            session.encoderToken=null;session.encoderReadyToken=null
            val frames=ScreenProjectionFrames(config.width,config.height,budget,{ _,pixels ->
                captureCurrent(session, identity, epoch) && pushStill(session,config,pixels)
            },{ reason -> encoderFailed(session,epoch,reason) })
            session.display = try {
                attachSurface(session, config, frames.surface)
            } catch (error: Throwable) {
                closeFailedCapture(error, frames)
            }
            session.frames=frames; session.encoder=null
            return@synchronized
        }
        val token=Any()
        session.encoderToken=token;session.encoderReadyToken=null
        val encoder = try {
            ScreenEncoder(config,budget) { buffer, offset, length, pts, key ->
                if (!captureCurrent(session,identity,epoch) || session.encoderToken!==token ||
                    (config.hybrid && session.encoderReadyToken!==token)) return@ScreenEncoder -1
                val flags = epochFlags(epoch) or (if (key) FLAG_KEYFRAME else 0) or (if(config.hybrid) 0x4 or (if(length==20) 0x10 else 0) else 0)
                NativeBridge.screenPushAdmitted(session.start.session,budget.generation,buffer,offset,length,pts,flags)
            }
        } catch (e: RuntimeException) {
            Log.w(TAG, "Kodierer nicht gestartet: ${e.message}")
            encoderFailed(session,epoch,e.message ?: "MediaCodec-Konfiguration abgelehnt")
            return@synchronized
        }
        encoder.setFailureHandler {reason->if(session.encoderToken===token && captureCurrent(session,identity,epoch)) encoderFailed(session,epoch,reason)}
        var newFrames: ScreenProjectionFrames?=null
        try {if (synthetic) {
            val lease=encoder.retainSurface()
            session.synthetic = try {SyntheticSurfaceSource(encoder.surface,config.width,config.height,config.fps,lease)}
                catch (failure: Throwable) {lease.close();throw failure}
        } else if (projection != null) {
            val frames=if(config.hybrid) ScreenProjectionFrames(config.width,config.height,budget,{ frame,pixels ->
                captureCurrent(session,identity,epoch) && session.encoderToken===token &&
                    session.encoderReadyToken===token && encoder.queueRgba(pixels,frame)
            },{ reason -> if(session.encoderToken===token && captureCurrent(session,identity,epoch)) encoderFailed(session,epoch,reason) }) else null
            newFrames=frames
            val output=frames?.surface ?: encoder.surface
            session.display=attachSurface(session,config,output)
            session.frames=frames
        }} catch (error: Throwable) {
            closeFailedCapture(error, newFrames, encoder)
        }
        session.encoder = encoder
        if(projection!=null && identity!=null) scope.launch {reportProjectionReady(session,identity,projection)}
    }

    private fun closeFailedCapture(
        failure: Throwable,
        frames: ScreenProjectionFrames?,
        encoder: ScreenEncoder? = null,
    ): Nothing {
        var interrupted = failure is InterruptedException
        fun retainCleanupFailure(cleanup: Throwable) {
            if (cleanup is InterruptedException) interrupted = true
            if (cleanup !== failure) {
                try {
                    failure.addSuppressed(cleanup)
                } catch (_: Throwable) {
                    Unit
                }
            }
        }
        try {
            frames?.close()
        } catch (cleanup: Throwable) {
            retainCleanupFailure(cleanup)
        }
        try {
            encoder?.release()
        } catch (cleanup: Throwable) {
            retainCleanupFailure(cleanup)
        }
        if (interrupted) Thread.currentThread().interrupt()
        throw failure
    }

    private fun retireEncoder(session: HostSession, source: SyntheticSurfaceSource?, encoder: ScreenEncoder?) {
        if (source == null) {encoder?.release();return}
        checkNotNull(encoder) {"Syntheticquelle ohne ihren Encoder"}
        session.encoderRetirements++
        ScreenSyntheticRetirement.retireSynthetic(source,encoder) { result ->
            synchronized(session.lock) {
                session.encoderRetirements--
                val failure=result.exceptionOrNull()
                if(failure!=null) {
                    session.pendingConfigStart=false
                    Log.e(TAG,"Syntheticbesitz bleibt bis zum tatsächlichen Ende gehalten",failure)
                    encoderFailed(session,session.config?.epoch ?: 0,"Syntheticabbau: ${failure.message}")
                } else if(session.encoderRetirements==0 && session.pendingConfigStart) {
                    session.pendingConfigStart=false
                    if(!session.ended && session.captureActive) try {applyConfig(session)}
                    catch(error: RuntimeException) {encoderFailed(session,session.config?.epoch ?: 0,
                        error.message ?: "Capturekonfiguration nach Abbau abgelehnt")}
                }
            }
        }
    }

    private fun captureCurrent(session: HostSession, identity: ScreenProjectionIdentity?, epoch: Int): Boolean {
        if (session.ended || !session.captureActive || session.config?.epoch != epoch) return false
        if (session.start.source == "synthetic") return true
        val projection = session.projection ?: return false
        return session.identity === identity && identity?.matches(projection) == true
    }

    private fun attachSurface(session: HostSession, config: EncoderConfig, surface: android.view.Surface): VirtualDisplay {
        check(session.captureActive) {"Capture ist pausiert"}
        val dpi=app.resources.displayMetrics.densityDpi
        val display=session.display
        if(display!=null) { display.resize(config.width,config.height,dpi); display.surface=surface; return display }
        return checkNotNull(requireNotNull(session.projection).createVirtualDisplay(
            "se-screen-${session.start.session}",
            config.width, config.height, dpi,
            DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, surface, null, null,
        )) {
            "Virtuelle Anzeige für die Bildschirmaufnahme konnte nicht erstellt werden"
        }
    }

    private fun pushStill(session: HostSession, config: EncoderConfig, pixels: ScreenCaptureBuffer): Boolean {
        if (!captureCurrent(session,session.identity,config.epoch)) return false
        val total=Math.addExact(pixels.capacity,8)
        return pixels.budget.buffer(ScreenCaptureBudget.TRANSPORT,total).use {packet -> packet.access {target ->
            target.putInt(config.width);target.putInt(config.height);pixels.access {target.put(it)}
            NativeBridge.screenPushAdmitted(session.start.session,pixels.budget.generation,target,0,total,0,epochFlags(config.epoch) or FLAG_STILL)>=0
        }}
    }

    private fun encoderFailed(session: HostSession, epoch: Int, reason: String) {
        Log.w(TAG,"Kodierer ${session.start.session}: $reason")
        val bytes=reason.toByteArray(Charsets.UTF_8).take(4096).toByteArray()
        val packet=java.nio.ByteBuffer.allocateDirect(bytes.size).put(bytes)
        NativeBridge.screenPush(session.start.session,packet,0,bytes.size,0,epochFlags(epoch) or 0x8)
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
        if (ScreenProjectionIdentity.requiresPackageProof(host.start.source)) {
            projection.stop()
            scope.launch { projectionFailed(session, ScreenProjectionIdentity.UNVERIFIED_APP) }
            gone()
            return
        }
        val identity = ScreenProjectionIdentity(projection)
        val attached = synchronized(host.lock) {
            if (host.ended || host.projection != null) false
            else { host.projection=projection; host.identity=identity; true }
        }
        if (!attached) {
            // One consent result creates one projection. Never replace an active grant silently.
            if (host.projection !== projection) projection.stop()
            gone()
            return
        }
        val callback = object : MediaProjection.Callback() {
                override fun onCapturedContentResize(width: Int, height: Int) {
                    if(host.identity === identity && identity.resized(projection,width,height) && !host.ended)
                        scope.launch { reportProjectionReady(host,identity,projection) }
                }
                override fun onStop() {
                    if (host.identity === identity && identity.matches(projection) && !host.ended) {
                        identity.invalidate()
                        val locked = context.getSystemService(KeyguardManager::class.java)?.isKeyguardLocked == true
                        scope.launch { endQuietly(session, if (locked) "host_locked" else "host_stopped") }
                        teardown(host)
                    }
                    gone()
                }
            }
        try {
            projection.registerCallback(callback,Handler(Looper.getMainLooper()))
        } catch (_: RuntimeException) {
            scope.launch { projectionFailed(session,"capture_failed") }
            teardown(host)
            gone()
            return
        }
        pendingConsent.remove(session)
        scope.launch { reportProjectionReady(host,identity,projection) }
        synchronized(host.lock) {
            try {
                applyConfig(host)
            } catch (error: RuntimeException) {
                encoderFailed(
                    host, host.config?.epoch ?: 0,
                    error.message ?: "Capturekonfiguration abgelehnt",
                )
            }
        }
        refreshNotification()
    }

    private suspend fun reportProjectionReady(host: HostSession, identity: ScreenProjectionIdentity, projection: MediaProjection) {
        host.readyLock.withLock {
            if (host.identity !== identity || !identity.matches(projection) || host.ended) return@withLock
            // Snapshot the exact configured encoder under the source lock, then release
            // that lock while the Native ready call is pending. A newer token cannot inherit it.
            val binding=synchronized(host.lock) {
                if(host.identity !== identity || !identity.matches(projection) || host.ended) null
                else Triple(identity.size() ?: displaySize(),host.encoder,host.encoderToken)
            } ?: return@withLock
            val actual=binding.second
            val best=ScreenEncoder.best() ?: ("mediacodec" to false)
            try {
                ScreenApi.ready(host.start.session,binding.first.first,binding.first.second,
                    actual?.name ?: best.first,actual?.hardware ?: best.second,true)
                synchronized(host.lock) {
                    if(host.identity===identity && identity.matches(projection) && !host.ended &&
                        host.encoder===actual && host.encoderToken===binding.third && actual!=null) {
                        host.encoderReadyToken=binding.third
                        if(host.captureActive) host.pendingCapture?.let {frame ->
                            host.frames?.let {frames -> host.pendingCapture=null;frames.capture(frame)}
                        }
                    }
                }
            } catch (_: CoreException) {teardown(host)}
        }
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
            source = if (it.identity != null) ScreenProjectionIdentity.SOURCE else it.start.source,
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
            session.still.close()
            session.identity?.invalidate()
            val source=session.synthetic;val encoder=session.encoder
            session.display?.release()
            session.frames?.close()
            if(source!=null) {
                session.synthetic=null;session.encoder=null
                retireEncoder(session,source,encoder)
            } else {encoder?.release();session.encoder=null}
            session.projection?.stop()
            session.synthetic = null
            session.display = null
            session.encoder = null
            session.frames = null
            session.projection = null
            session.identity = null
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
