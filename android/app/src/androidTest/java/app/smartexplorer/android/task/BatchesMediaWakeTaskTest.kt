package app.smartexplorer.android.task

import android.app.UiAutomation
import android.content.pm.ActivityInfo
import android.graphics.Bitmap
import android.graphics.Color
import android.os.ParcelFileDescriptor
import android.view.SurfaceView
import android.view.View
import android.view.ViewGroup
import android.view.inspector.WindowInspector
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.Lifecycle
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.smartexplorer.android.MainActivity
import app.smartexplorer.android.ui.viewer.ImagePage
import app.smartexplorer.android.ui.viewer.LocalMedia
import app.smartexplorer.android.ui.viewer.MediaPlaybackState
import app.smartexplorer.android.ui.viewer.PlayerPage
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Actual Android holder wake requests; no mocked player, sensor or physical timeout claim. */
@RunWith(AndroidJUnit4::class)
class BatchesMediaWakeTaskTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()

    private fun <T> ui(block: () -> T): T {
        val result = AtomicReference<T>()
        compose.runOnUiThread { result.set(block()) }
        return result.get()
    }

    private fun content(block: @Composable () -> Unit) {
        ui { compose.activity.setContent { MaterialTheme { block() } } }
        compose.waitForIdle()
    }

    @Composable
    private fun Preview(block: @Composable () -> Unit) {
        Dialog({}, properties = DialogProperties(usePlatformDefaultWidth = false)) {
            Box(Modifier.fillMaxSize()) { block() }
        }
    }

    @Composable
    private fun Player(file: File, audio: Boolean = false) {
        PlayerPage(LocalMedia(file.path, if (audio) "audio/wav" else "video/mp4"),
            file.name, audio, true, true, {}, { Text("wake-player-failed: $it") })
    }

    private fun video(): File {
        val path = InstrumentationRegistry.getArguments().getString("batchesVideoPath")
            ?: throw AssertionError("Task runner must provide the existing MP4 >= 8s as batchesVideoPath")
        return File(path).also { assertTrue("Missing media fixture: $path", it.isFile) }
    }

    // A media preview is a separate Dialog window. Inspect every actual process window,
    // not just MainActivity.decorView, and read View state only on its UI thread.
    private fun views(): List<View> = ui {
        val result = mutableListOf<View>()
        fun visit(view: View) {
            result += view
            if (view is ViewGroup) for (index in 0 until view.childCount) visit(view.getChildAt(index))
        }
        WindowInspector.getGlobalWindowViews().forEach { visit(it) }
        result
    }

    private fun surface(): SurfaceView {
        compose.waitUntil(15000) {
            views().filterIsInstance<SurfaceView>().any { ui { it.isAttachedToWindow && it.holder.surface.isValid } }
        }
        return views().filterIsInstance<SurfaceView>()
            .single { ui { it.isAttachedToWindow && it.holder.surface.isValid } }
    }

    private fun wake(view: SurfaceView, expected: Boolean) {
        compose.waitUntil(5000) { ui { view.keepScreenOn == expected } }
    }

    private fun noWake() {
        compose.waitUntil(5000) {
            val current = views()
            ui { current.none { it.keepScreenOn } }
        }
    }

    private fun playing() {
        compose.waitUntil(15000) {
            compose.onAllNodesWithContentDescription("Pause").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText("wake-player-failed", substring = true).assertDoesNotExist()
    }

    private fun slider() = compose.onNode(SemanticsMatcher.keyIsDefined(SemanticsProperties.ProgressBarRangeInfo))

    private fun position(): Float = slider().fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo].current

    private fun seek(ms: Float) {
        slider().performSemanticsAction(SemanticsActions.SetProgress) { it(ms) }
        compose.waitForIdle()
    }

    @Test
    fun batches_android_video_wake_follows_play_pause_completion_and_close() {
        val file = video()
        try {
            content { Preview { Player(file) } }
            playing()
            val holderView = surface()
            wake(holderView, true)
            compose.onNodeWithContentDescription("Pause").performClick()
            wake(holderView, false)
            seek(3000f)
            compose.onNodeWithContentDescription("Abspielen").performClick()
            playing()
            wake(holderView, true)
            val duration = slider().fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo].range.endInclusive
            assertTrue("Fixture must be at least eight seconds", duration >= 8000f)
            seek(duration - 500f)
            compose.waitUntil(20000) {
                compose.onAllNodesWithContentDescription("Abspielen").fetchSemanticsNodes().isNotEmpty()
            }
            wake(holderView, false)
            compose.onNodeWithContentDescription("Abspielen").performClick()
            playing()
            wake(holderView, true)
            content { }
            wake(holderView, false)
            noWake()
        } finally { content { } }
    }

    private fun shell(command: String): String {
        val automation = InstrumentationRegistry.getInstrumentation()
            .getUiAutomation(UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)
        return ParcelFileDescriptor.AutoCloseInputStream(automation.executeShellCommand(command))
            .bufferedReader().use { it.readText().trim() }
    }

    private fun rotate(value: Int) {
        shell("settings put system user_rotation $value")
        compose.waitUntil(10000) { ui { compose.activity.display!!.rotation == value } }
        compose.waitForIdle()
    }

    private fun withSystemRotation(block: () -> Unit) {
        val auto = shell("settings get system accelerometer_rotation")
        val user = shell("settings get system user_rotation")
        try {
            shell("settings put system accelerometer_rotation 0")
            rotate(0)
            block()
        } finally {
            for ((name, value) in listOf("user_rotation" to user, "accelerometer_rotation" to auto)) {
                require(value == "null" || value.toIntOrNull() != null)
                shell(if (value == "null") "settings delete system $name" else "settings put system $name $value")
            }
        }
    }

    @Test
    fun batches_android_video_wake_preserves_system_rotation_and_explicit_background_resume() = withSystemRotation {
        val file = video()
        val owner = compose.activity
        try {
            content { Preview { Player(file) } }
            playing()
            wake(surface(), true)
            compose.onNodeWithContentDescription("Pause").performClick()
            seek(3000f)
            rotate(1)
            assertSame(owner, compose.activity)
            assertEquals(ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED, ui { owner.requestedOrientation })
            compose.onNodeWithContentDescription("Abspielen").assertExists()
            assertEquals(3000f, position(), 300f)
            noWake()
            seek(0f)
            compose.onNodeWithContentDescription("Abspielen").performClick()
            playing()
            rotate(0)
            assertSame(owner, compose.activity)
            compose.onNodeWithContentDescription("Pause").assertExists()
            val beforeStop = position()
            val holderView = surface()
            wake(holderView, true)
            compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
            wake(holderView, false)
            compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
            compose.waitForIdle()
            compose.onNodeWithContentDescription("Abspielen").assertExists()
            assertTrue("Background return lost the retained position", position() >= beforeStop - 300f)
            noWake()
            compose.onNodeWithContentDescription("Abspielen").performClick()
            playing()
            wake(surface(), true)
        } finally {
            compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
            content { }
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
        return File.createTempFile("batches-wake-audio", ".wav", compose.activity.cacheDir)
            .also { it.writeBytes(buffer.array()) }
    }

    @Test
    fun batches_android_media_switch_to_image_audio_or_error_releases_video_wake() {
        val video = video()
        val audio = wav()
        val image = File.createTempFile("batches-wake-image", ".png", compose.activity.cacheDir)
        val mode = mutableStateOf("video")
        val missing = File(compose.activity.cacheDir, "batches-wake-missing-${System.nanoTime()}.mp4")
        try {
            val bitmap = Bitmap.createBitmap(32, 32, Bitmap.Config.ARGB_8888).apply { eraseColor(Color.BLUE) }
            try {
                image.outputStream().use { assertTrue(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) }
            } finally { bitmap.recycle() }
            content {
                Preview {
                    key(mode.value) {
                        when (mode.value) {
                            "video" -> Player(video)
                            "audio" -> Player(audio, audio = true)
                            "image" -> ImagePage(LocalMedia(image.path, "image/png"), true, {}, {}, { Text("wake-image-failed") })
                            else -> Player(missing)
                        }
                    }
                }
            }
            playing()
            val oldSurface = surface()
            wake(oldSurface, true)
            ui { mode.value = "image" }
            compose.waitForIdle()
            wake(oldSurface, false)
            noWake()
            compose.waitUntil(10000) {
                compose.onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsProperties.ProgressBarRangeInfo))
                    .fetchSemanticsNodes().isEmpty()
            }
            compose.onNodeWithText("wake-image-failed").assertDoesNotExist()
            ui { mode.value = "audio" }
            playing()
            noWake()
            assertTrue(views().filterIsInstance<SurfaceView>().isEmpty())
            ui { mode.value = "missing" }
            compose.waitUntil(15000) {
                compose.onAllNodesWithText("wake-player-failed", substring = true).fetchSemanticsNodes().isNotEmpty()
            }
            noWake()
        } finally {
            content { }
            audio.delete()
            image.delete()
        }
    }

    @Test
    fun batches_android_surface_wake_moves_without_reset_and_ignores_stale_detach() {
        val file = video()
        val surfaces = mutableListOf<SurfaceView>()
        var playback: MediaPlaybackState? = null
        val prepared = AtomicBoolean(false)
        val error = AtomicReference<String?>(null)
        try {
            content {
                Column {
                    repeat(2) {
                        AndroidView(factory = { context -> SurfaceView(context).also { view -> surfaces += view } },
                            modifier = Modifier.fillMaxWidth().height(120.dp))
                    }
                }
            }
            compose.waitUntil(15000) { ui { surfaces.size == 2 && surfaces.all { it.holder.surface.isValid } } }
            val first = ui { surfaces[0] }
            val second = ui { surfaces[1] }
            ui {
                playback = MediaPlaybackState(compose.activity.applicationContext).also { state ->
                    state.display(first.holder)
                    state.player.setOnPreparedListener { prepared.set(true) }
                    state.player.setOnErrorListener { _, what, extra -> error.set("$what/$extra"); true }
                    state.player.setDataSource(file.path)
                    state.player.isLooping = true
                    state.player.prepareAsync()
                }
            }
            compose.waitUntil(15000) { prepared.get() || error.get() != null }
            assertNull(error.get())
            val state = playback ?: throw AssertionError("No platform player")
            noWake()
            ui { assertFalse(state.start(visible = false)) }
            noWake()
            ui { assertTrue(state.start(visible = true)) }
            wake(first, true)
            compose.waitUntil(5000) { ui { state.player.currentPosition > 500 } }
            val position = ui { state.player.currentPosition }
            ui { state.display(second.holder) }
            wake(first, false)
            wake(second, true)
            ui {
                state.clearDisplay(first.holder)
                assertTrue(state.player.isPlaying)
                assertTrue(state.player.currentPosition >= position - 100)
            }
            wake(second, true)
            ui { state.pause() }
            wake(second, false)
            ui { assertTrue(state.start(visible = true)) }
            wake(second, true)
            ui { state.clearDisplay(second.holder) }
            noWake()
            ui { state.display(first.holder) }
            wake(first, true)
            ui { state.release(); state.release(); state.clearDisplay(first.holder); state.display(second.holder) }
            wake(first, false)
            noWake()
            assertNull(error.get())
        } finally {
            ui { playback?.release() }
            content { }
        }
    }
}
