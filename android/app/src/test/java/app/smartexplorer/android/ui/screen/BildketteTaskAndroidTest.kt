package app.smartexplorer.android.ui.screen

import app.smartexplorer.android.core.ScreenCursorEvent
import app.smartexplorer.android.screen.ScreenCodecLatency
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The Android part of the picture chain (plan 2026-11-14 `bildkette.md`, package BK7a): encoder hints, viewer rate, pointer. */
class BildketteTaskAndroidTest {
    @Test
    fun bildkette_task_android_encoder_attempts_fall_back_to_today() {
        // Non-hybrid pictures try the new hints first; a refusal moves on to today's settings in their old order.
        assertEquals(
            listOf(ScreenCodecLatency.EncoderHints.MODERN, ScreenCodecLatency.EncoderHints.TODAY_REALTIME, ScreenCodecLatency.EncoderHints.TODAY_PLAIN),
            ScreenCodecLatency.encoderHintTiers(hybrid = false),
        )
        // Hybrid pictures keep today's attempts only.
        assertEquals(
            listOf(ScreenCodecLatency.EncoderHints.TODAY_REALTIME, ScreenCodecLatency.EncoderHints.TODAY_PLAIN),
            ScreenCodecLatency.encoderHintTiers(hybrid = true),
        )
    }

    @Test
    fun bildkette_task_android_keyframes_are_requested_only_in_the_new_attempt() {
        // A negative interval means no key frame after the first one; today's attempts keep their interval.
        assertEquals(-1, ScreenCodecLatency.keyframeIntervalS(ScreenCodecLatency.EncoderHints.MODERN, 10))
        assertEquals(10, ScreenCodecLatency.keyframeIntervalS(ScreenCodecLatency.EncoderHints.TODAY_REALTIME, 10))
        assertEquals(10, ScreenCodecLatency.keyframeIntervalS(ScreenCodecLatency.EncoderHints.TODAY_PLAIN, 10))
    }

    @Test
    fun bildkette_task_android_intra_refresh_needs_the_codec_feature_and_api_24() {
        assertEquals(60, ScreenCodecLatency.intraRefreshPeriod(fps = 60, sdk = 30, featureSupported = true))
        assertNull(ScreenCodecLatency.intraRefreshPeriod(fps = 60, sdk = 30, featureSupported = false))
        assertNull(ScreenCodecLatency.intraRefreshPeriod(fps = 60, sdk = 23, featureSupported = true))
    }

    @Test
    fun bildkette_task_android_latency_hints_follow_their_api_levels() {
        // KEY_LATENCY (encoder) from API 26, KEY_LOW_LATENCY (decoder) from API 30.
        assertFalse(ScreenCodecLatency.encoderLatencyAllowed(25))
        assertTrue(ScreenCodecLatency.encoderLatencyAllowed(26))
        assertFalse(ScreenCodecLatency.decoderLatencyAllowed(29))
        assertTrue(ScreenCodecLatency.decoderLatencyAllowed(30))
    }

    @Test
    fun bildkette_task_android_picture_rate_counts_pictures_per_second() {
        var now = 0L
        val meter = PictureRateMeter { now }
        // The first picture only opens the window; 50 pictures 20 ms apart end it at 50 per second.
        assertNull(meter.picture())
        val first = (1..50).map { i -> now = i * 20_000_000L; meter.picture() }
        assertEquals(listOf(null), first.dropLast(1).distinct())
        assertEquals(50, first.last())
        // The next window counts from the end of the last one: 25 pictures 40 ms apart make 25 per second.
        val second = (1..25).map { j -> now = 1_000_000_000L + j * 40_000_000L; meter.picture() }
        assertEquals(25, second.last())
    }

    @Test
    fun bildkette_task_android_pointer_is_hidden_only_while_the_overlay_draws() {
        val visibility = HostPointerVisibility()
        val shape7 = ScreenCursorEvent.Shape(1L, 7L, 1, 1, 0, 0, 1, ByteArray(4))
        val shape8 = ScreenCursorEvent.Shape(1L, 8L, 1, 1, 0, 0, 1, ByteArray(4))
        fun position(shapeId: Long, visible: Boolean = true) = ScreenCursorEvent.Position(1L, shapeId, visible, 0, 0, 100, 100)
        // A position without a shape draws nothing; the shape that the position names completes the overlay.
        assertFalse(visibility.update(position(7L)))
        assertTrue(visibility.update(shape7))
        // A new shape that the position does not name yet hides the overlay again.
        assertFalse(visibility.update(shape8))
        assertTrue(visibility.update(position(8L)))
        assertFalse(visibility.update(position(8L, visible = false)))
    }
}
