package app.smartexplorer.android.screen

import android.media.MediaFormat
import app.smartexplorer.android.ui.screen.ScreenPresentedFrame
import app.smartexplorer.android.ui.screen.surfaceFrameContinues
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** Decoder output geometry and the viewer's per-frame readiness rule. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = android.app.Application::class)
class ScreenDecoderOutputTest {
    private fun output(width: Int, height: Int, crop: IntArray? = null) = MediaFormat.createVideoFormat("video/avc", width, height).apply {
        if (crop != null) {
            setInteger("crop-left", crop[0]); setInteger("crop-top", crop[1])
            setInteger("crop-right", crop[2]); setInteger("crop-bottom", crop[3])
        }
    }

    @Test fun screen_metadata_runtime_task_decoderOutputMayPadBeyondOneMacroblock() {
        // A window of 1234×816 in a frame a decoder aligned to 64: MediaCodec bounds no padding.
        ScreenFrameLimits.checkOutput(output(1280, 832, intArrayOf(0, 0, 1233, 815)), 1234, 816)
        ScreenFrameLimits.checkOutput(output(1248, 816, intArrayOf(0, 0, 1233, 815)), 1234, 816)
        // Without crop keys the picture is the whole frame.
        ScreenFrameLimits.checkOutput(output(1920, 1080), 1920, 1080)
    }

    @Test fun screen_metadata_runtime_task_decoderOutputStillRefusesAnotherPicture() {
        assertThrows(IllegalArgumentException::class.java) {
            ScreenFrameLimits.checkOutput(output(1280, 832, intArrayOf(0, 0, 1199, 815)), 1234, 816)
        }
        assertThrows(IllegalArgumentException::class.java) { ScreenFrameLimits.checkOutput(output(1920, 1088), 1920, 1080) }
        assertThrows(IllegalArgumentException::class.java) { ScreenFrameLimits.checkOutput(output(1200, 816), 1234, 816) }
    }

    @Test fun screen_metadata_runtime_task_surfaceFramesOfOneGroupKeepInputReadiness() {
        val first = ScreenPresentedFrame(session = 3, gopId = 40, frameId = 7, epoch = 2)
        assertTrue(surfaceFrameContinues(first, first.copy(frameId = 8), bitmapShown = false))
        assertFalse(surfaceFrameContinues(null, first, bitmapShown = false))
        assertFalse(surfaceFrameContinues(first, first.copy(frameId = 8), bitmapShown = true))
        assertFalse(surfaceFrameContinues(first, first.copy(gopId = 41, frameId = 1), bitmapShown = false))
        assertFalse(surfaceFrameContinues(first, first.copy(epoch = 3), bitmapShown = false))
        assertFalse(surfaceFrameContinues(first, first.copy(session = 4), bitmapShown = false))
    }
}
