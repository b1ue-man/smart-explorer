package app.smartexplorer.android.task

import android.app.UiAutomation
import android.graphics.Bitmap
import android.graphics.Color
import android.os.ParcelFileDescriptor
import android.view.View
import android.widget.RemoteViews
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.smartexplorer.android.MainActivity
import app.smartexplorer.android.api.ScreenBufferBudget
import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.ui.DragOrigin
import app.smartexplorer.android.launcher.ui.DragState
import app.smartexplorer.android.launcher.ui.Hover
import app.smartexplorer.android.launcher.widgets.LauncherWidgetHostView
import app.smartexplorer.android.ui.screen.*
import app.smartexplorer.android.ui.viewer.ImagePage
import app.smartexplorer.android.ui.viewer.LocalMedia
import app.smartexplorer.android.ui.viewer.PlayerPage
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicBoolean
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Real Android rendering/player ownership. System rotation here is not a sensor-lock test. */
@RunWith(AndroidJUnit4::class)
class BatchesPresentationTaskTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()

    private fun content(block: @Composable () -> Unit) {
        // MainActivity owns configChanges. Replace its content on that same production Activity.
        compose.runOnUiThread { compose.activity.setContent { MaterialTheme { block() } } }
        compose.waitForIdle()
    }

    private fun shell(command: String): String {
        val automation = InstrumentationRegistry.getInstrumentation()
            .getUiAutomation(UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)
        return ParcelFileDescriptor.AutoCloseInputStream(automation.executeShellCommand(command))
            .bufferedReader().use { it.readText().trim() }
    }

    private fun rotation(): Int {
        var value = -1
        compose.runOnUiThread { value = compose.activity.display!!.rotation }
        return value
    }

    private fun rotateTo(value: Int) {
        shell("settings put system user_rotation $value")
        compose.waitUntil(10000) { rotation() == value }
        compose.waitForIdle()
    }

    private fun withSystemRotation(block: () -> Unit) {
        val auto = shell("settings get system accelerometer_rotation")
        val user = shell("settings get system user_rotation")
        try {
            shell("settings put system accelerometer_rotation 0")
            rotateTo(0)
            block()
        } finally {
            for ((key, value) in listOf("user_rotation" to user, "accelerometer_rotation" to auto)) {
                require(value == "null" || value.toIntOrNull() != null)
                shell(if (value == "null") "settings delete system $key" else "settings put system $key $value")
            }
        }
    }

    @Test
    fun batches_android_image_zoom_survives_system_layout_change() = withSystemRotation {
        val file = File.createTempFile("batches-image", ".png", compose.activity.cacheDir)
        val image = Bitmap.createBitmap(480, 320, Bitmap.Config.ARGB_8888)
        image.eraseColor(Color.BLUE)
        file.outputStream().use { assertTrue(image.compress(Bitmap.CompressFormat.PNG, 100, it)) }
        image.recycle()
        val zoomed = AtomicBoolean(false)
        val active = mutableStateOf(true)
        try {
            content {
                Box(Modifier.fillMaxSize().testTag("batches-image")) {
                    ImagePage(LocalMedia(file.path, "image/png"), active.value, {},
                        { zoomed.set(it) }, { Text("image-decode-failed") })
                }
            }
            compose.waitUntil(10000) {
                compose.onAllNodes(isRoot()).fetchSemanticsNodes().isNotEmpty() &&
                    compose.onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsProperties.ProgressBarRangeInfo)).fetchSemanticsNodes().isEmpty()
            }
            compose.onNodeWithText("image-decode-failed").assertDoesNotExist()
            compose.onNodeWithTag("batches-image").performTouchInput { doubleClick(center) }
            compose.waitUntil(5000) { zoomed.get() }
            val before = compose.onNodeWithTag("batches-image").fetchSemanticsNode().boundsInRoot
            rotateTo(1)
            val after = compose.onNodeWithTag("batches-image").fetchSemanticsNode().boundsInRoot
            assertTrue("OS rotation did not relayout the media page", before.width != after.width && before.height != after.height)
            // Force the existing page to report its retained state after recomposition.
            compose.runOnUiThread { active.value = false }
            compose.waitForIdle()
            zoomed.set(false)
            compose.runOnUiThread { active.value = true }
            compose.waitUntil(5000) { zoomed.get() }
            compose.onNodeWithTag("batches-image").performTouchInput { doubleClick(center) }
            compose.waitUntil(5000) { !zoomed.get() }
        } finally {
            content { }
            file.delete()
        }
    }

    private fun wav(): File {
        val rate = 8000
        val bytes = rate * 12 * 2
        val buffer = ByteBuffer.allocate(44 + bytes).order(ByteOrder.LITTLE_ENDIAN)
        buffer.put("RIFF".toByteArray()).putInt(36 + bytes).put("WAVEfmt ".toByteArray())
        buffer.putInt(16).putShort(1).putShort(1).putInt(rate).putInt(rate * 2)
        buffer.putShort(2).putShort(16).put("data".toByteArray()).putInt(bytes)
        repeat(bytes / 2) { buffer.putShort(0) }
        return File.createTempFile("batches-audio", ".wav", compose.activity.cacheDir)
            .also { it.writeBytes(buffer.array()) }
    }

    private fun playerRotation(file: File, audio: Boolean) = withSystemRotation {
        assertTrue("Missing media fixture: ${file.path}", file.isFile)
        val originalActivity = compose.activity
        content {
            Box(Modifier.fillMaxSize().testTag("batches-player")) {
                PlayerPage(LocalMedia(file.path, if (audio) "audio/wav" else "video/mp4"),
                    file.name, audio, true, true, {}, { Text("player-failed: $it") })
            }
        }
        try {
            compose.waitUntil(15000) {
                compose.onAllNodesWithContentDescription("Pause").fetchSemanticsNodes().isNotEmpty()
            }
            compose.onNodeWithContentDescription("Pause").performClick()
            val slider = compose.onNode(SemanticsMatcher.keyIsDefined(SemanticsProperties.ProgressBarRangeInfo))
            val range = slider.fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo].range
            assertTrue("Fixture must be at least eight seconds", range.endInclusive >= 8000f)
            slider.performSemanticsAction(SemanticsActions.SetProgress) { it(3000f) }
            compose.waitForIdle()
            val before = compose.onNodeWithTag("batches-player").fetchSemanticsNode().boundsInRoot
            rotateTo(1)
            val after = compose.onNodeWithTag("batches-player").fetchSemanticsNode().boundsInRoot
            assertSame("Production configChanges must retain this player owner", originalActivity, compose.activity)
            assertTrue(before.width != after.width && before.height != after.height)
            compose.onNodeWithContentDescription("Abspielen").assertExists()
            val paused = slider.fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo].current
            assertEquals("Paused position survived layout change", 3000f, paused, 300f)
            compose.onNodeWithContentDescription("Abspielen").performClick()
            compose.waitUntil(5000) {
                slider.fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo].current > paused + 500f
            }
            rotateTo(0)
            compose.onNodeWithContentDescription("Pause").assertExists()
            assertTrue(slider.fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo].current > paused)
        } finally { content { } }
    }

    @Test
    fun batches_android_audio_pause_seek_and_resume_survive_system_layout_change() {
        val audio = wav()
        try { playerRotation(audio, true) } finally { audio.delete() }
    }

    @Test
    fun batches_android_video_pause_seek_and_resume_survive_system_layout_change() {
        val path = InstrumentationRegistry.getArguments().getString("batchesVideoPath")
            ?: throw AssertionError("Task runner must generate and push an MP4 >= 8s and set batchesVideoPath")
        playerRotation(File(path), false)
    }

    @Test
    fun batches_android_widget_drag_uses_pixels_from_the_attached_host_once() {
        lateinit var host: LauncherWidgetHostView
        content {
            AndroidView(factory = { context ->
                LauncherWidgetHostView(context).also {
                    host = it
                    it.updateAppWidget(RemoteViews("android", android.R.layout.simple_list_item_1).apply {
                        setTextViewText(android.R.id.text1, "Actual widget pixels")
                        setInt(android.R.id.text1, "setBackgroundColor", Color.BLUE)
                        setTextColor(android.R.id.text1, Color.WHITE)
                    })
                }
            }, modifier = Modifier.size(240.dp, 120.dp))
        }
        compose.runOnIdle {
            assertTrue(host.isAttachedToWindow && host.isLaidOut)
            assertNotNull(host.findViewById<View>(android.R.id.text1))
            val drag = DragState()
            drag.begin(HomeItem(id = "batches-widget", kind = "widget"), Offset.Zero, DragOrigin.Home)
            var captures = 0
            drag.armWidgetSnapshot("batches-widget") { captures++; host.dragSnapshot()?.asImageBitmap() }
            drag.move(Offset(2f, 0f), Hover.None, 0, 8f)
            assertEquals(0, captures)
            assertNull(drag.widgetPreview)
            drag.move(Offset(20f, 0f), Hover.None, 0, 8f)
            assertEquals(1, captures)
            val snapshot = drag.widgetPreview ?: throw AssertionError("No real widget snapshot")
            val bitmap = snapshot.asAndroidBitmap()
            assertEquals(host.width, bitmap.width)
            assertEquals(host.height, bitmap.height)
            var bluePixels = 0
            for (y in 0 until bitmap.height step 4) for (x in 0 until bitmap.width step 4) {
                if (bitmap.getPixel(x, y) == Color.BLUE) bluePixels++
            }
            assertTrue("Snapshot omitted the RemoteViews content", bluePixels > 100)
            drag.move(Offset(40f, 20f), Hover.None, 0, 8f)
            assertSame(snapshot, drag.widgetPreview)
            assertEquals(1, captures)
            drag.finish()
            assertNull(drag.widgetPreview)
            assertFalse(drag.active)
        }
        content { }
        compose.runOnIdle { assertNull(host.dragSnapshot()) }
    }

    @Test
    fun batches_android_scaler_preserves_identity_and_bounds_pending_outputs() {
        val source = Bitmap.createBitmap(64, 64, Bitmap.Config.ARGB_8888).apply { eraseColor(Color.RED) }
        val imageBytes = 64 * 64 * 4
        val budget = ScreenBufferBudget(1024 * 1024L, imageBytes, 512, imageBytes, imageBytes + 32)
        val delivered = CopyOnWriteArrayList<ScreenScaled?>()
        val identity = ScreenPresentedFrame(17, (1L shl 40) + 7, (1L shl 42) + 9, 3)
        lateinit var worker: ScreenScaleWorker
        var workerInitialized = false
        lateinit var owner: ScreenScaleWorker.Owner
        lateinit var previewOwner: ScreenScaleWorker.Owner
        try {
            compose.runOnUiThread {
                worker = ScreenScaleWorker { budget }
                workerInitialized = true
                owner = worker.owner()
                previewOwner = worker.owner()
                worker.submit(owner, source, listOf(32, 32, 0, 0, 32, 32), identity) { delivered += it; true }
            }
            compose.waitUntil(10000) { delivered.size == 1 }
            val first = delivered.single() ?: throw AssertionError("Admitted scale failed")
            assertEquals(identity, first.identity)
            assertEquals(32, first.bitmap.width)
            assertEquals(Color.RED, first.bitmap.getPixel(16, 16))
            compose.runOnUiThread {
                worker.submit(owner, source, listOf(16, 16, 0, 0, 16, 16), identity.copy(frameId = identity.frameId + 1)) { delivered += it; true }
                worker.submit(previewOwner, source, listOf(8, 8, 0, 0, 8, 8), identity.copy(frameId = identity.frameId + 2)) { delivered += it; true }
                assertEquals("Unapplied output must hold back the next CPU job", 1, delivered.size)
                first.applied.complete(Unit)
            }
            compose.waitUntil(10000) { delivered.size == 2 }
            val latest = delivered.last() ?: throw AssertionError("Conflated scale failed")
            assertEquals(8, latest.bitmap.width)
            assertEquals(identity.frameId + 2, latest.identity!!.frameId)
            compose.runOnUiThread {
                assertTrue(owner.retainedBytes > 0 && previewOwner.retainedBytes > 0)
                worker.detach(owner)
                worker.detach(previewOwner)
                first.close()
                latest.close()
                assertEquals(0L, owner.retainedBytes)
                assertEquals(0L, previewOwner.retainedBytes)
                val nextOwner = worker.owner()
                worker.submit(nextOwner, source, listOf(1024, 1024, 0, 0, 1024, 1024), identity) { delivered += it; true }
            }
            compose.waitUntil(10000) { delivered.size == 3 }
            assertNull("Unaffordable viewport must not allocate/deliver an output", delivered.last())
            compose.runOnUiThread {
                worker.close()
            }
        } finally {
            compose.runOnUiThread {
                if (workerInitialized) worker.close()
                delivered.filterNotNull().forEach { it.close() }
            }
            source.recycle()
        }
    }

    @Test
    fun batches_android_bitmap_reports_the_identity_from_its_actual_draw() {
        val source = Bitmap.createBitmap(64, 64, Bitmap.Config.ARGB_8888).apply { eraseColor(Color.GREEN) }
        val budget = ScreenBufferBudget(1024 * 1024L, 16384, 512, 16384, 16416)
        val worker = ScreenScaleWorker { budget }
        val drawn = CopyOnWriteArrayList<ScreenPresentedFrame>()
        val identity = ScreenPresentedFrame(19, (1L shl 40) + 3, (1L shl 42) + 11, 5)
        try {
            assertTrue(drawn.isEmpty())
            content {
                ScreenBitmap(source, 1, 64, 64, Modifier.testTag("batches-bitmap"),
                    identity = identity, scaler = worker, onPresented = { drawn += it })
            }
            compose.waitUntil(10000) { drawn.isNotEmpty() }
            assertTrue(drawn.all { it == identity })
            val output = compose.onNodeWithTag("batches-bitmap").captureToImage().asAndroidBitmap()
            assertEquals(Color.GREEN, output.getPixel(output.width / 2, output.height / 2))
            content { }
            compose.runOnIdle { worker.close() }
        } finally {
            compose.runOnUiThread { worker.close() }
            source.recycle()
        }
    }

    @Test
    fun batches_android_native_pixel_crop_and_resample_obey_budget_geometry() {
        val source = Bitmap.createBitmap(4, 1, Bitmap.Config.ARGB_8888)
        val budget = ScreenBufferBudget(1024 * 1024L, 16384, 512, 16384, 16416)
        source.setPixels(intArrayOf(Color.RED, Color.GREEN, Color.BLUE, Color.WHITE), 0, 4, 0, 0, 4, 1)
        try {
            val crop = ScreenResample.plan(source, 4, 1, 1, 0, 2, 1, budget)
            assertTrue(crop.nativePixels)
            assertFalse(crop.alias)
            assertEquals(0L, crop.rowBytes)
            assertEquals(2L * crop.outputBytes + 1024, crop.workingBytes)
            val output = ScreenResample.resize(source, crop) { }
            try {
                assertEquals(Color.GREEN, output.getPixel(0, 0))
                assertEquals(Color.BLUE, output.getPixel(1, 0))
            } finally { output.recycle() }
            val alias = ScreenResample.plan(source, 4, 1, 0, 0, 4, 1, budget)
            assertSame(source, ScreenResample.resize(source, alias) { })
            assertThrows(IllegalArgumentException::class.java) {
                ScreenResample.plan(source, 4, 1, 3, 0, 2, 1, budget)
            }
        } finally { source.recycle() }
    }

}
