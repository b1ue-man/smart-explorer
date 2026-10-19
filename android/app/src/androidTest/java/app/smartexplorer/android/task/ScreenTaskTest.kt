package app.smartexplorer.android.task

import android.app.Activity
import android.app.PendingIntent
import android.graphics.Bitmap
import android.graphics.PixelFormat
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
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.test.junit4.createAndroidComposeRule
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
import app.smartexplorer.android.core.ScreenHostStart
import app.smartexplorer.android.screen.EncoderConfig
import app.smartexplorer.android.screen.GestureMapper
import app.smartexplorer.android.screen.ScreenConsentActivity
import app.smartexplorer.android.screen.ScreenControlService
import app.smartexplorer.android.screen.ScreenEncoder
import app.smartexplorer.android.screen.ScreenHostControl
import app.smartexplorer.android.screen.ScreenHostNotifier
import app.smartexplorer.android.screen.ScreenHostService
import app.smartexplorer.android.screen.ScreenStillSource
import app.smartexplorer.android.screen.SyntheticSurfaceSource
import app.smartexplorer.android.service.approvalNotification
import app.smartexplorer.android.system.Notifications
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import org.junit.Assert.assertEquals
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

    private fun shell(command: String): String {
        val output = InstrumentationRegistry.getInstrumentation().uiAutomation.executeShellCommand(command)
        return android.os.ParcelFileDescriptor.AutoCloseInputStream(output).bufferedReader().readText()
    }

    /** Switches the se accessibility service on (the suite's device, never the user's). */
    private fun accessibilityOn(): ScreenControlService {
        val name = "${appContext.packageName}/${ScreenControlService::class.java.name}"
        shell("settings put secure enabled_accessibility_services $name")
        shell("settings put secure accessibility_enabled 1")
        val deadline = SystemClock.uptimeMillis() + 15_000
        while (SystemClock.uptimeMillis() < deadline) {
            ScreenControlService.current()?.let { return it }
            SystemClock.sleep(200)
        }
        throw AssertionError("Bedienungshilfe nicht verbunden")
    }

    /** One still of the accessibility service, after the 333 ms interval. */
    private fun still(service: ScreenControlService): Bitmap {
        var shot: Bitmap? = null
        val done = CountDownLatch(1)
        SystemClock.sleep(ScreenStillSource.MIN_INTERVAL_MS)
        service.still { bitmap ->
            shot = bitmap
            done.countDown()
        }
        assertTrue("Kein Standbild", done.await(5, TimeUnit.SECONDS))
        return shot ?: throw AssertionError("Android lieferte kein Standbild")
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
        var firstKey: ByteArray? = null
        val config = EncoderConfig(epoch = 1, width = 1280, height = 720, fps = 30, bitrate = 4_000_000, profile = "high")
        val encoder = ScreenEncoder(config) { buffer, offset, length, _, key ->
            synchronized(pushes) {
                if (key && firstKey == null) {
                    firstKey = ByteArray(minOf(length, 64)).also { bytes ->
                        val view = buffer.duplicate()
                        view.position(offset)
                        view.get(bytes)
                    }
                }
                pushes += length to key
            }
            0
        }
        val source = SyntheticSurfaceSource(encoder.surface, config.width, config.height, config.fps)
        val started = SystemClock.uptimeMillis()
        SystemClock.sleep(3_000)
        source.stop()
        encoder.release()
        val seconds = (SystemClock.uptimeMillis() - started) / 1000.0
        val frames = synchronized(pushes) { pushes.toList() }
        val fps = frames.size / seconds
        val kbps = frames.sumOf { it.first } * 8 / 1000.0 / seconds
        TaskReport.note("screen.host.bench", "encoder=${encoder.name} hardware=${encoder.hardware} fps=%.1f kbps=%.0f frames=${frames.size}".format(fps, kbps))
        assertTrue("zu wenige Bilder: ${frames.size}", frames.size >= 45)
        assertTrue("erstes Bild kein Schlüsselbild", frames.first().second)
        // Annex B, SPS (NAL type 7) in front of the key frame.
        val head = firstKey ?: throw AssertionError("kein Schlüsselbild")
        assertEquals(listOf<Byte>(0, 0, 0, 1), head.take(4))
        assertEquals(7, head[4].toInt() and 0x1F)
        // The core knows the phone as host with this encoder.
        val prereqs = Api.obj(
            "screen.hostPrereqs",
            args("encoders" to listOf(mapOf("profile" to "high", "hardware" to encoder.hardware)), "display" to mapOf("width" to 1080, "height" to 2400)),
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
        val reader = ImageReader.newInstance(width, height, PixelFormat.RGBA_8888, 2)
        val display = running.createVirtualDisplay("se-probe", width, height, 160, DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, reader.surface, null, null)
        var image = reader.acquireLatestImage()
        val deadline = SystemClock.uptimeMillis() + 5_000
        while (image == null && SystemClock.uptimeMillis() < deadline) {
            SystemClock.sleep(100)
            image = reader.acquireLatestImage()
        }
        assertNotNull("kein Bild der Projektion", image)
        val plane = image!!.planes[0]
        val bitmap = Bitmap.createBitmap(plane.rowStride / plane.pixelStride, height, Bitmap.Config.ARGB_8888)
        bitmap.copyPixelsFromBuffer(plane.buffer)
        image.close()
        display.release()
        running.stop()
        reader.close()
        ScreenHostControl.probe = null
        val dark = darkShare(bitmap, 0, 0, width, height)
        TaskReport.note("screen.host.snapshot", "size=${width}x$height dark=%.2f".format(dark))
        assertTrue("Projektion nur schwarz", dark < 0.95)
    }

    @Test
    fun screen_task_android_still_interval_respected() {
        val service = accessibilityOn()
        val times = mutableListOf<Long>()
        val stills = ScreenStillSource { _, length ->
            synchronized(times) { times += SystemClock.uptimeMillis() }
            if (length > ScreenStillSource.HEADER) 0 else -2
        }
        repeat(4) {
            val done = CountDownLatch(1)
            var code: Int? = null
            stills.capture(service) {
                code = it
                done.countDown()
            }
            assertTrue("Standbild ohne Antwort", done.await(5, TimeUnit.SECONDS))
            assertEquals(0, code)
        }
        val gaps = times.zipWithNext { a, b -> b - a }
        TaskReport.note("screen.host.still", "gaps=$gaps")
        assertTrue("Abstand unter 333 ms: $gaps", gaps.all { it >= ScreenStillSource.MIN_INTERVAL_MS - 5 })
    }

    @Test
    fun screen_task_android_still_secure_window_black() {
        val service = accessibilityOn()
        var top = 0
        compose.setContent { Box(Modifier.fillMaxSize().onGloballyPositioned { top = it.boundsInWindow().top.toInt() }) { Text("Geheim") } }
        compose.runOnUiThread { compose.activity.window.addFlags(WindowManager.LayoutParams.FLAG_SECURE) }
        compose.waitForIdle()
        SystemClock.sleep(500)
        val shot = still(service)
        compose.runOnUiThread { compose.activity.window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE) }
        val dark = darkShare(shot, 0, top + shot.height / 4, shot.width, shot.height * 3 / 4)
        TaskReport.note("screen.host.secure", "dark=%.2f".format(dark))
        assertTrue("sicheres Fenster nicht schwarz: $dark", dark > 0.9)
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
}
