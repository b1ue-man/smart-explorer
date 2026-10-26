package app.smartexplorer.android.ui.viewer

import androidx.compose.ui.geometry.Offset
import org.junit.Assert.*
import org.junit.Test

/** Geometry oracle only; physical rotation lock requires a sensor-capable device run. */
class BatchesMediaStateTest {
    @Test
    fun batches_android_rotation_keeps_image_focus_in_image_coordinates() {
        val portrait = MediaGeometry.fit(1200, 800, 400f, 800f)
        val landscape = MediaGeometry.fit(1200, 800, 800f, 400f)
        val focus = Offset(0.6f, 0.55f)
        for (geometry in listOf(portrait, landscape, portrait)) {
            val restored = geometry.focus(4f, geometry.offset(4f, focus))
            assertEquals(focus.x, restored.x, 0.0001f)
            assertEquals(focus.y, restored.y, 0.0001f)
        }
        assertEquals(Offset.Zero, portrait.offset(1f, focus))
        assertEquals(Offset.Zero, landscape.offset(1f, focus))
    }

    @Test
    fun batches_android_letterbox_pan_is_clamped_to_the_fitted_image() {
        val fitted = MediaGeometry.fit(1600, 400, 400f, 800f)
        assertEquals(400f, fitted.width, 0f)
        assertEquals(100f, fitted.height, 0f)
        assertEquals(Offset(200f, 0f), fitted.clamp(Offset(10000f, 10000f), 2f))
        assertEquals(Offset(-200f, 0f), fitted.clamp(Offset(-10000f, -10000f), 2f))
    }
}
