package app.smartexplorer.android.task

import android.app.Activity
import android.app.PendingIntent
import android.app.UiAutomation
import android.graphics.Bitmap
import android.graphics.ImageFormat
import android.hardware.display.DisplayManager
import android.media.ImageReader
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.SystemClock
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.result.ActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import app.smartexplorer.android.api.AccessApproval
import app.smartexplorer.android.api.CAP_SCREEN_CONTROL
import app.smartexplorer.android.api.CAP_SCREEN_VIEW
import app.smartexplorer.android.api.DECISION_ALWAYS
import app.smartexplorer.android.api.DECISION_ONCE
import app.smartexplorer.android.api.DecoderOffer
import app.smartexplorer.android.api.ScreenDevice
import app.smartexplorer.android.api.ScreenProbe
import app.smartexplorer.android.core.ScreenHostStart
import app.smartexplorer.android.core.ScreenViewEvent
import app.smartexplorer.android.screen.EncoderConfig
import app.smartexplorer.android.screen.GestureMapper
import app.smartexplorer.android.screen.ScreenConsentActivity
import app.smartexplorer.android.screen.ScreenControlService
import app.smartexplorer.android.screen.ScreenEncoder
import app.smartexplorer.android.screen.ScreenHostControl
import app.smartexplorer.android.screen.ScreenHostNotifier
import app.smartexplorer.android.screen.ScreenHostService
import app.smartexplorer.android.screen.ScreenStillSource
import app.smartexplorer.android.screen.ScreenCaptureBudget
import app.smartexplorer.android.screen.ScreenCaptureBitmap
import app.smartexplorer.android.screen.ScreenProjectionFrames
import app.smartexplorer.android.core.NativeBridge
import app.smartexplorer.android.screen.SyntheticSurfaceSource
import app.smartexplorer.android.service.approvalNotification
import app.smartexplorer.android.ui.screen.LocalScreenBackend
import app.smartexplorer.android.ui.screen.PictureSource
import app.smartexplorer.android.ui.screen.ScreenBackend
import app.smartexplorer.android.ui.screen.ScreenDecoder
import app.smartexplorer.android.ui.screen.ScreenTarget
import app.smartexplorer.android.ui.screen.ScreenTilesScreen
import app.smartexplorer.android.system.Notifications
import java.nio.ByteOrder
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.serialization.json.Json
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * SCR-M8 on the device (plan 2026-10-17 „Rechte, se Desktop, Tresore“, strang-bildschirm.md B 391):
 * this phone as screen host. The Rust side of the bridge runs as host tests
 * (`screen_task_android_bridge_shapes`, `bridge_tests.rs`); here the app's parts run for real:
 * MediaCodec with the test picture, MediaProjection after the system dialog (the suite allows
 * `PROJECT_MEDIA` with appops), the accessibility service's stills, gestures and text, and the
 * consent actions as activity intents. No second device takes part.
 */
@RunWith(AndroidJUnit4::class)
class ScreenTaskTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private val device: UiDevice
        get() = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())

    private val question = AccessApproval(
        id = "fedcba9876543210fedcba9876543210",
        deviceName = "Laptop",
        deviceId = "d-laptop",
        fingerprint = "fp-laptop-0123456789",
        capability = CAP_SCREEN_VIEW,
        capabilityLabel = "Bildschirm ansehen",
        text = "Laptop möchte den Bildschirm ansehen",
        surface = "notification",
    )

    /**
     * The suite's UiAutomation. `Instrumentation.getUiAutomation()` is `getUiAutomation(0)`, and a
     * UiAutomation without `FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES` makes the system unbind every
     * other accessibility service and bind none for as long as it is connected
     * (`UiAutomationManager.suppressingAccessibilityServicesLocked`). A call with other flags
     * reconnects the same instance with those flags: every shell command of this class goes through
     * here and never through `uiAutomation`.
     */
    private fun automation(): UiAutomation =
        InstrumentationRegistry.getInstrumentation().getUiAutomation(UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)

    private fun shell(command: String): String {
        val output = automation().executeShellCommand(command)
        return android.os.ParcelFileDescriptor.AutoCloseInputStream(output).bufferedReader().readText()
    }

    /** The lines of `dumpsys accessibility` that list services: enabled, bound, binding, crashed. */
    private fun accessibilityLines(): List<String> {
        val lists = listOf("Enabled services:", "Bound services:", "Binding services:", "Crashed services:")
        return shell("dumpsys accessibility").lineSequence()
            .map { it.trim() }
            .filter { line -> lists.any { line.startsWith(it) } }
            .toList()
    }

    /** Whether the system still lists the se service as enabled or as crashed. */
    private fun systemListsService(): Boolean = accessibilityLines().any { line ->
        (line.startsWith("Enabled services:") || line.startsWith("Crashed services:")) &&
            line.contains(ScreenControlService::class.java.simpleName)
    }

    /** The connected se service: waits up to [timeoutMs] for it and gives `null` when there is none. */
    private fun connectedService(timeoutMs: Long): ScreenControlService? {
        val deadline = SystemClock.uptimeMillis() + timeoutMs
        do {
            ScreenControlService.current()?.let { return it }
            SystemClock.sleep(100)
        } while (SystemClock.uptimeMillis() < deadline)
        return ScreenControlService.current()
    }

    /**
     * Switches the se accessibility service on and returns the connected service (the suite's
     * device, never the user's). In an instrumentation run two things of the system keep it away.
     * First, a UiAutomation without flags suppresses it ([automation]). Second, `am instrument`
     * force-stops the app when it starts and when it ends: the dead binding puts the service on the
     * system's crashed list, `AccessibilityManagerService.updateServicesLocked` skips crashed
     * services, and only a service that left the enabled list is taken off that list again („users
     * may toggle the on/off switch to retry“). Writing the same value again changes nothing, so
     * the setting is deleted, the system's answer is awaited, and then the service is put back.
     */
    private fun accessibilityOn(): ScreenControlService {
        automation()
        connectedService(1_500)?.let { return it }
        val name = "${appContext.packageName}/${ScreenControlService::class.java.name}"
        shell("settings delete secure enabled_accessibility_services")
        val deadline = SystemClock.uptimeMillis() + 10_000
        while (systemListsService() && SystemClock.uptimeMillis() < deadline) SystemClock.sleep(200)
        shell("settings put secure enabled_accessibility_services $name")
        shell("settings put secure accessibility_enabled 1")
        return connectedService(15_000)
            ?: throw AssertionError("Bedienungshilfe nicht verbunden: ${accessibilityLines()}")
    }

    /** One still of the accessibility service, after the 333 ms interval. */
    private fun still(service: ScreenControlService, fixture: ScreenCaptureFixture): ScreenCaptureBitmap {
        var shot: ScreenCaptureBitmap? = null
        var expired = false
        val lock = Any()
        val done = CountDownLatch(1)
        SystemClock.sleep(ScreenStillSource.MIN_INTERVAL_MS)
        service.still(fixture.budget(),fixture.width,fixture.height) { bitmap ->
            synchronized(lock) {
                if(expired) bitmap?.close() else shot=bitmap
                done.countDown()
            }
        }
        val answered=done.await(5,TimeUnit.SECONDS)
        return synchronized(lock) {
            expired=true
            if(!answered) {shot?.close();shot=null;throw AssertionError("Kein Standbild; später Callback bleibt Besitzer")}
            shot.also {shot=null} ?: throw AssertionError("Android lieferte kein Standbild")
        }
    }

    private fun displaySize(): Pair<Int,Int> {
        val bounds=appContext.getSystemService(WindowManager::class.java).maximumWindowMetrics.bounds
        return bounds.width() to bounds.height()
    }

    private fun darkShare(bitmap: Bitmap, left: Int, top: Int, right: Int, bottom: Int): Double {
        var dark = 0
        var total = 0
        for (y in top until bottom step 8) {
            for (x in left until right step 8) {
                val pixel = bitmap.getPixel(x, y)
                val sum = (pixel shr 16 and 0xFF) + (pixel shr 8 and 0xFF) + (pixel and 0xFF)
                if (sum < 30) dark++
                total++
            }
        }
        return dark.toDouble() / total.coerceAtLeast(1)
    }

    @Test
    fun screen_task_android_host_bench_synthetic() = coreTest {
        val pushes = mutableListOf<Pair<Int, Boolean>>()
        var firstPrefix: List<Byte>? = null
        var firstNal: Int? = null
        val config = EncoderConfig(epoch = 1, width = 1280, height = 720, fps = 30, bitrate = 4_000_000, profile = "high")
        lateinit var encoderName:String
        var hardware=false
        var started=0L
        ScreenCaptureFixture.open(config.width,config.height,ScreenCaptureBudget.SYNTHETIC_SURFACE).use { fixture ->
            var encoder:ScreenEncoder?=null
            var source:SyntheticSurfaceSource?=null
            try {
                val opened=ScreenEncoder(config,fixture.budget()) { buffer, offset, length, _, key ->
                    synchronized(pushes) {
                        if(key && firstPrefix==null) {
                            require(length>=5)
                            firstPrefix=(0 until 4).map {buffer.get(offset+it)}
                            firstNal=buffer.get(offset+4).toInt() and 0x1F
                        }
                        pushes += length to key
                    }
                    0
                }
                encoder=opened;encoderName=opened.name;hardware=opened.hardware
                source=fixture.source(opened,config)
                started=SystemClock.uptimeMillis()
                SystemClock.sleep(3_000)
            } finally {fixture.retireSynthetic(source,encoder)}
        }
        val seconds = (SystemClock.uptimeMillis() - started) / 1000.0
        val frames = synchronized(pushes) { pushes.toList() }
        val fps = frames.size / seconds
        val kbps = frames.sumOf { it.first } * 8 / 1000.0 / seconds
        TaskReport.note("screen.host.bench", "encoder=$encoderName hardware=$hardware fps=%.1f kbps=%.0f frames=${frames.size}".format(fps, kbps))
        assertTrue("zu wenige Bilder: ${frames.size}", frames.size >= 45)
        assertTrue("erstes Bild kein Schlüsselbild", frames.first().second)
        assertEquals(listOf<Byte>(0, 0, 0, 1), firstPrefix ?: throw AssertionError("kein Schlüsselbild"))
        assertEquals(7,firstNal)
        val prereqs = Api.obj(
            "screen.hostPrereqs",
            args("encoders" to listOf(mapOf("profile" to "high", "hardware" to hardware)), "display" to mapOf("width" to 1080, "height" to 2400)),
        )
        TaskReport.note("screen.hostPrereqs", prereqs.toString())
        assertTrue(Api.obj("screen.hostSessions").array("sessions").isEmpty())
    }

    @Test
    fun screen_task_android_host_snapshot_nonblack() {
        shell("appops set ${appContext.packageName} PROJECT_MEDIA allow")
        val projections = CountDownLatch(1)
        var projection: MediaProjection? = null
        ScreenHostControl.probe = {
            projection = it
            projections.countDown()
        }
        try {
            val results = CountDownLatch(1)
            var result: ActivityResult? = null
            compose.activityRule.scenario.onActivity { activity ->
                val manager = activity.getSystemService(MediaProjectionManager::class.java)
                val launcher = activity.activityResultRegistry.register("screen-probe", ActivityResultContracts.StartActivityForResult()) {
                    result = it
                    results.countDown()
                }
                launcher.launch(manager.createScreenCaptureIntent())
            }
            // Without appops the dialog asks: the suite's device answers „Start now“ / „Jetzt starten“.
            if (!results.await(3, TimeUnit.SECONDS)) {
                device.wait(Until.findObject(By.text(java.util.regex.Pattern.compile("(?i)(start now|jetzt starten|starten|start)"))), 5_000)?.click()
                assertTrue("Systemabfrage ohne Antwort", results.await(20, TimeUnit.SECONDS))
            }
            val answer = result ?: throw AssertionError("kein Ergebnis")
            assertEquals(Activity.RESULT_OK, answer.resultCode)
            ScreenHostService.start(appContext, ScreenHostControl.PROBE_SESSION, answer.resultCode, answer.data!!)
            assertTrue("keine Projektion", projections.await(10, TimeUnit.SECONDS))
            val running = projection ?: throw AssertionError("keine Projektion")
            val bounds = appContext.getSystemService(WindowManager::class.java).maximumWindowMetrics.bounds
            val width = bounds.width() / 4
            val height = bounds.height() / 4
            ScreenCaptureFixture.open(width,height,ScreenCaptureBudget.PROJECTION_TILES).use { fixture ->
                val done=CountDownLatch(1)
                val failure=java.util.concurrent.atomic.AtomicReference<String?>()
                var dark:Double?=null
                val frames=ScreenProjectionFrames(width,height,fixture.budget(),{_,_->
                    failure.set("Unexpected regular Projection submit");done.countDown();false
                },{reason->failure.set(reason);done.countDown()})
                var display:android.hardware.display.VirtualDisplay?=null
                try {
                    frames.captureStill(1) { pixels ->
                        dark=pixels.access { bytes ->
                            var black=0;var total=0
                            for(y in 0 until height step 8) for(x in 0 until width step 8) {
                                val at=(y*width+x)*4
                                val sum=(bytes.get(at).toInt() and 255)+(bytes.get(at+1).toInt() and 255)+(bytes.get(at+2).toInt() and 255)
                                if(sum<30) black++
                                total++
                            }
                            black.toDouble()/total.coerceAtLeast(1)
                        }
                        done.countDown();true
                    }
                    display=running.createVirtualDisplay("se-probe",width,height,160,DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,frames.surface,null,null)
                    assertTrue("kein Bild der Projektion",done.await(5,TimeUnit.SECONDS))
                    failure.get()?.let {throw AssertionError(it)}
                    val observed=dark ?: throw AssertionError("keine Projektionspixel")
                    TaskReport.note("screen.host.snapshot","size=${width}x$height dark=%.2f".format(observed))
                    assertTrue("Projektion nur schwarz",observed<0.95)
                } finally {
                    captureCleanup({display?.release()},{check(frames.close()) {"Projection handler has not ended"}},{running.stop()})
                }
            }
        } finally {ScreenHostControl.probe=null;projection?.stop()}
    }

    @Test
    fun screen_task_android_still_interval_respected() {
        val service = accessibilityOn()
        // The connection may have answered a still of an earlier test a moment ago.
        SystemClock.sleep(2 * ScreenStillSource.MIN_INTERVAL_MS)
        val requested = mutableListOf<Long>()
        val (width,height)=displaySize()
        ScreenCaptureFixture.open(width,height,ScreenCaptureBudget.ACCESSIBILITY).use { fixture ->
            val failure=java.util.concurrent.atomic.AtomicReference<Throwable?>()
            val stills=ScreenStillSource(fixture.session,{width to height}) {buffer,length,generation ->
                try {
                    require(length==ScreenStillSource.HEADER+Math.multiplyExact(Math.multiplyExact(width,height),4))
                    require(buffer.order(ByteOrder.LITTLE_ENDIAN).getInt(0)==width && buffer.getInt(4)==height)
                    val code=NativeBridge.screenPushAdmitted(fixture.session,generation,buffer,0,length,0L,0x2)
                    check(code==0) {"Admitted still push refused: $code"}
                    val consumed=fixture.consumeStill(generation)
                    check(consumed.int("length")==length)
                    0
                } catch(error:Throwable) {failure.set(error);-2}
            }
            try {
                repeat(4) { round ->
                    val done = CountDownLatch(1)
                    var code: Int? = null
                    stills.capture(service) {code=it;done.countDown()}
                    requested += SystemClock.uptimeMillis()
                    assertTrue("Standbild ${round + 1} ohne Antwort", done.await(5, TimeUnit.SECONDS))
                    failure.get()?.let {throw AssertionError("Real still transfer failed",it)}
                    assertEquals("Standbild ${round + 1}: Android lieferte keins (Anfragen $requested)", 0, code)
                }
            } finally {stills.close()}
        }
        val gaps = requested.zipWithNext { a, b -> b - a }
        TaskReport.note("screen.host.still", "gaps=$gaps")
        assertTrue("Abstand unter 333 ms: $gaps", gaps.all { it >= ScreenStillSource.MIN_INTERVAL_MS })
    }

    @Test
    fun screen_task_android_still_secure_window_black() {
        val service = accessibilityOn()
        var top = 0
        compose.setContent { Box(Modifier.fillMaxSize().onGloballyPositioned { top = it.boundsInWindow().top.toInt() }) { Text("Geheim") } }
        compose.runOnUiThread { compose.activity.window.addFlags(WindowManager.LayoutParams.FLAG_SECURE) }
        try {
            compose.waitForIdle()
            SystemClock.sleep(500)
            val (width,height)=displaySize()
            ScreenCaptureFixture.open(width,height,ScreenCaptureBudget.ACCESSIBILITY).use { fixture ->
                still(service,fixture).use {shot ->
                    val dark = darkShare(shot.bitmap, 0, top + shot.bitmap.height / 4, shot.bitmap.width, shot.bitmap.height * 3 / 4)
                    TaskReport.note("screen.host.secure", "dark=%.2f".format(dark))
                    assertTrue("sicheres Fenster nicht schwarz: $dark", dark > 0.9)
                }
            }
        } finally {compose.runOnUiThread {compose.activity.window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)}}
    }

    @Test
    fun screen_task_android_gesture_tap_lands() {
        val service = accessibilityOn()
        var taps by mutableStateOf(0)
        var center = 0f to 0f
        compose.setContent {
            Box(
                Modifier.fillMaxSize()
                    .onGloballyPositioned {
                        val box = it.boundsInWindow()
                        center = box.center.x to box.center.y
                    }
                    .clickable { taps++ },
            )
        }
        compose.waitForIdle()
        val bounds = appContext.getSystemService(WindowManager::class.java).maximumWindowMetrics.bounds
        val location = IntArray(2)
        compose.runOnUiThread { compose.activity.window.decorView.getLocationOnScreen(location) }
        val x = ((location[0] + center.first) / (bounds.width() - 1) * 65_535).toInt()
        val y = ((location[1] + center.second) / (bounds.height() - 1) * 65_535).toInt()
        val events = Json.parseToJsonElement(
            """[{"type":"pointer_abs","x":$x,"y":$y},{"type":"button","button":"left","down":true},{"type":"button","button":"left","down":false}]""",
        ) as JsonArray
        val mapper = GestureMapper({ bounds.width() }, { bounds.height() })
        val done = CountDownLatch(1)
        var completed = false
        GestureMapper.perform(service, mapper.map(events, now = 0).single()) {
            completed = it
            done.countDown()
        }
        assertTrue(done.await(5, TimeUnit.SECONDS))
        assertTrue("Geste abgebrochen", completed)
        compose.waitUntil(5_000) { taps == 1 }
        TaskReport.note("screen.host.tap", "x=$x y=$y")
    }

    @Test
    fun screen_task_android_text_set() {
        val service = accessibilityOn()
        var text by mutableStateOf("")
        val focus = FocusRequester()
        compose.setContent {
            Column {
                TextField(value = text, onValueChange = { text = it }, modifier = Modifier.fillMaxWidth().height(64.dp).focusRequester(focus))
            }
        }
        compose.runOnIdle { focus.requestFocus() }
        compose.waitForIdle()
        val events = Json.parseToJsonElement("""[{"type":"text","text":"hallo"}]""") as JsonArray
        val mapper = GestureMapper({ 1 }, { 1 })
        val done = CountDownLatch(1)
        var set = false
        GestureMapper.perform(service, mapper.map(events).single()) {
            set = it
            done.countDown()
        }
        assertTrue(done.await(5, TimeUnit.SECONDS))
        assertTrue("ACTION_SET_TEXT abgelehnt", set)
        compose.waitUntil(5_000) { text == "hallo" }
    }

    /** The PendingIntent of [action] is an activity intent of [intent] (works on API 30 too). */
    private fun assertActivityIntent(actual: PendingIntent, code: Int, intent: android.content.Intent) {
        val existing = PendingIntent.getActivity(appContext, code, intent, PendingIntent.FLAG_NO_CREATE or PendingIntent.FLAG_IMMUTABLE)
        assertNotNull("kein Activity-PendingIntent", existing)
        assertEquals(existing, actual)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) assertTrue(actual.isActivity)
        assertEquals(ScreenConsentActivity::class.java.name, intent.component?.className)
    }

    @Test
    fun screen_task_android_consent_action_is_activity_intent() {
        val start = ScreenHostStart(41, "monitor:0", false, 1080, 2400, "Laptop", "d-laptop")
        val notification = ScreenHostNotifier.consentNotification(appContext, start)
        val titles = notification.actions.orEmpty().map { it.title.toString() }
        assertEquals(listOf("Freigeben", "Ablehnen"), titles)
        assertActivityIntent(notification.actions[0].actionIntent, ScreenHostNotifier.consentId(41), ScreenConsentActivity.sessionIntent(appContext, start))
        assertEquals(notification.actions[0].actionIntent, notification.contentIntent)
        // „Ablehnen“ ends the session without opening anything.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) assertTrue(notification.actions[1].actionIntent.isService)
    }

    @Test
    fun perm_task_android_screen_approval_actions_are_activity_intents() {
        Notifications.ensureChannels(appContext)
        for (capability in listOf(CAP_SCREEN_VIEW, CAP_SCREEN_CONTROL)) {
            val approval = question.copy(capability = capability)
            val id = Notifications.ID_APPROVAL_FIRST + 2
            val notification = approvalNotification(appContext, approval, id)
            val titles = notification.actions.orEmpty().map { it.title.toString() }
            assertEquals(listOf("Diesmal erlauben", "Immer erlauben", "Ablehnen"), titles)
            for ((index, decision) in listOf(DECISION_ONCE, DECISION_ALWAYS).withIndex()) {
                val action = notification.actions[index]
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) assertTrue(action.isAuthenticationRequired)
                val code = id * 4 + if (decision == DECISION_ALWAYS) 1 else 0
                assertActivityIntent(action.actionIntent, code, ScreenConsentActivity.approvalIntent(appContext, approval, decision))
            }
            // „Ablehnen“ stays the broadcast that answers only this question.
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) assertTrue(notification.actions[2].actionIntent.isBroadcast)
        }
        TaskReport.note("access_approval.screen", "actions=activity")
    }

    /** The barcode of the test picture (32 columns, bit 31 left, in the upper eighth) in the luma of [image]. */
    private fun barcode(image: android.media.Image): Long {
        val plane = image.planes[0]
        val row = image.height / 16
        var value = 0L
        for (bit in 0 until 32) {
            val x = ((bit + 0.5) * image.width / 32).toInt()
            val luma = plane.buffer.get(row * plane.rowStride + x * plane.pixelStride).toInt() and 0xFF
            value = value shl 1 or (if (luma > 128) 1L else 0L)
        }
        return value
    }

    /**
     * SCR-M9 on the device: the encoder of the host side draws the test picture, its access units
     * go through the pull layout (16-byte head and full GOP/frame IDs, little endian, the first picture marked as the
     * start of a group) into the viewer's MediaCodec decoder, and the barcode is read from what the
     * decoder rendered. The decoder also offers `tiles`.
     */
    @Test
    fun screen_task_android_viewer_decodes_barcode() {
        assertTrue(ScreenDecoder.offers().any { it.codec == "tiles" })
        val config = EncoderConfig(epoch = 1, width = 1280, height = 720, fps = 30, bitrate = 4_000_000, profile = "high")
        val fixture=ScreenCaptureFixture.open(config.width,config.height,ScreenCaptureBudget.SYNTHETIC_SURFACE,true)
        val fullGop = (1L shl 40) + 1
        val frameBase = 1L shl 42
        var units:ScreenCaptureQueue?=null
        var encoder:ScreenEncoder?=null
        var source:SyntheticSurfaceSource?=null
        var reader:ImageReader?=null
        var decoder:ScreenDecoder?=null
        var pulls:app.smartexplorer.android.ui.screen.ScreenPullBuffers?=null
        var decoderStarted=false
        val ended=CountDownLatch(1)
        val presentations = CopyOnWriteArrayList<app.smartexplorer.android.ui.screen.ScreenPresentedFrame>()
        try {
            val queue=ScreenCaptureQueue(fixture.budget(),fullGop,frameBase);units=queue
            val opened=ScreenEncoder(config,fixture.budget()) {buffer,offset,length,pts,key -> queue.push(buffer,offset,length,pts,key)}
            encoder=opened
            val synthetic=fixture.source(opened,config);source=synthetic
            val pictures = PictureSource {dst,timeoutMs -> queue.pull(dst,timeoutMs)}
            val images=ImageReader.newInstance(config.width,config.height,ImageFormat.YUV_420_888,3);reader=images
            val fixedPulls=fixture.pulls();pulls=fixedPulls
            val rendered = java.util.concurrent.atomic.AtomicInteger()
            val stream = ScreenViewEvent.Stream(1, "h264", "high", config.width, config.height, fullGop, opened.name, opened.hardware)
            val activeDecoder = ScreenDecoder(
                sessionId = 1,
                source = pictures,
                streamOf = { if (it == fullGop) stream else null },
                surface = { images.surface },
                listener = object : ScreenDecoder.Listener {
                    override fun onSize(width: Int, height: Int) = Unit
                    override fun onBitmap(bitmap: Bitmap) = Unit
                    override fun onPicture() {
                        rendered.incrementAndGet()
                    }
                    override fun onSurfaceFrame(frame: app.smartexplorer.android.ui.screen.ScreenPresentedFrame) { presentations += frame }
                    override fun onDecodeFailure(consecutive: Int) = Unit
                    override fun onEnded() {ended.countDown()}
                },
                bufferBudget={fixture.viewerBudget},pullBuffers=fixedPulls,
            )
            decoder=activeDecoder
            activeDecoder.start();decoderStarted=true
            val deadline = SystemClock.uptimeMillis() + 15_000
            while (rendered.get() < 10 && SystemClock.uptimeMillis() < deadline) SystemClock.sleep(50)
            assertTrue("zu wenige Bilder dekodiert: ${rendered.get()}", rendered.get() >= 10)
            val one=checkNotNull(images.acquireLatestImage()) {"kein Bild des Dekoders"}.use {barcode(it)}
            val seen = rendered.get()
            while (rendered.get() < seen + 10 && SystemClock.uptimeMillis() < deadline) SystemClock.sleep(50)
            val two=checkNotNull(images.acquireLatestImage()) {"kein zweites Bild"}.use {barcode(it)}
            TaskReport.note("screen.viewer.barcode", "frames=${synthetic.frames} first=$one second=$two")
            assertTrue("Strichcode $one außerhalb der gezeichneten Bilder", one in 0..synthetic.frames)
            assertTrue("Strichcode steigt nicht: $one → $two", two > one && two <= synthetic.frames)
            val presentationDeadline = SystemClock.uptimeMillis() + 5_000
            while (presentations.isEmpty() && SystemClock.uptimeMillis() < presentationDeadline) SystemClock.sleep(10)
            assertTrue("No actual Surface presentation identity", presentations.isNotEmpty())
            assertTrue(presentations.all { it.session == 1L && it.gopId == fullGop && it.frameId > frameBase && it.epoch == 0L })
            queue.checkHealthy()
        } finally {
            var decoderEnded=!decoderStarted
            var readerEnded=reader==null
            captureCleanup(
                {units?.close()},
                {decoder?.stop();if(decoderStarted) {check(ended.await(5,TimeUnit.SECONDS)) {"Decoder has not relinquished its pull/codec owners"};decoderEnded=true}},
                {fixture.retireSynthetic(source,encoder)},
                {if(decoderEnded) {reader?.close();reader=null;readerEnded=true;fixture.releaseReader()}},
                {if(decoderEnded && readerEnded) {pulls?.relinquishFixedBuffer();pulls=null;fixture.releaseViewer()}},
                {fixture.close()},
            )
        }
    }

    /** A backend without a core: one device that allows its screen, and the calls the screens make. */
    private class FakeScreenBackend : ScreenBackend {
        val calls = CopyOnWriteArrayList<String>()
        private val flow = MutableSharedFlow<ScreenViewEvent>(extraBufferCapacity = 64)
        override val events: Flow<ScreenViewEvent> get() = flow

        override fun offers() = listOf(DecoderOffer(codec = "tiles"))

        override fun pictures(session: Long) = PictureSource { _, timeoutMs ->
            SystemClock.sleep(minOf(timeoutMs, 50).toLong())
            -1
        }

        override suspend fun devices(roomId: String?) =
            listOf(ScreenDevice("direct", "c1", "d1", "Laptop", online = true, supported = true, view = "allow", control = "allow"))

        override suspend fun probe(target: ScreenTarget) = ScreenProbe(device = "Laptop", supported = true, platform = "windows", view = "allow", control = "allow")

        override suspend fun open(target: ScreenTarget, mode: String, source: String?, displayWidth: Int, displayHeight: Int, maxFps: Int, decoders: List<DecoderOffer>): Long {
            calls += "open:$mode"
            // The first events come before the call returns, as with the core.
            val body = buildJsonObject { putJsonObject("input") { put("pointer", true) } }
            flow.tryEmit(ScreenViewEvent.Accepted(7, "direct", "windows", "allow", "allow", body))
            flow.tryEmit(ScreenViewEvent.State(7, "streaming", null))
            return 7
        }

        override suspend fun setMode(session: Long, mode: String, source: String?, width: Int?, height: Int?, fps: Int?, sharpness: Boolean, lossless: Boolean) {
            calls += "setMode:$session:$mode"
        }

        override suspend fun input(session: Long, events: List<JsonObject>) = events.size.also { calls += "input:$session" }

        override suspend fun control(session: Long, action: String) {
            calls += "control:$session:$action"
        }

        override suspend fun listWindows(session: Long) {
            calls += "listWindows:$session"
        }

        override suspend fun keyframe(session: Long) {
            calls += "keyframe:$session"
        }

        override suspend fun close(session: Long) {
            calls += "close:$session"
        }
    }

    /**
     * SCR-M9, group view to full view and back: a tap on a tile raises the same session from the
     * preview to the full stream (no second `viewOpen`), closing the full view puts it back to the
     * preview, and only leaving the grid ends the session.
     */
    @Test
    fun screen_task_android_grid_switch() {
        val backend = FakeScreenBackend()
        var showGrid by mutableStateOf(true)
        compose.setContent {
            CompositionLocalProvider(LocalScreenBackend provides backend) {
                if (showGrid) ScreenTilesScreen(null) { showGrid = false }
            }
        }
        compose.waitUntil(10_000) { backend.calls.contains("open:preview") }
        compose.waitUntil(10_000) { backend.calls.any { it.startsWith("setMode:7:preview") } }
        compose.onNodeWithText("Laptop").performClick()
        compose.waitUntil(10_000) { backend.calls.contains("setMode:7:full") }
        // The calls in the message: the run shows which one is the second `open`.
        assertEquals("a second connection was opened: ${backend.calls}", 1, backend.calls.count { it.startsWith("open:") })
        compose.onNodeWithTag("screen_view_close").performClick()
        compose.waitUntil(10_000) { backend.calls.lastIndexOf("setMode:7:preview") > backend.calls.indexOf("setMode:7:full") }
        assertFalse("the preview session ended with the full view: ${backend.calls}", backend.calls.contains("close:7"))
        compose.onNodeWithTag("screen_grid_close").performClick()
        compose.waitUntil(10_000) { backend.calls.contains("close:7") }
        assertEquals("the session was closed more than once: ${backend.calls}", 1, backend.calls.count { it == "close:7" })
        assertEquals("a second connection was opened: ${backend.calls}", 1, backend.calls.count { it.startsWith("open:") })
        TaskReport.note("screen.grid.switch", backend.calls.joinToString(" "))
    }
}
