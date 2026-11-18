package app.smartexplorer.android.ui.screen

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * „Monitor“ and „Auflösung“ of the Android viewer (plan 2026-11-17 P5/P9): stored values, typed sizes and the
 * arguments of `screen.viewSetDisplay`. Pure functions only.
 */
class ScreenDisplayChoiceTest {
    @Test
    fun p9_task_android_display_choice_reads_stored_and_typed_values() {
        // A stored choice: an unknown wish is automatic, a size needs both sides within 1..16384.
        assertEquals(ViewerDisplayChoice(), ViewerDisplayChoice.of(null, null, null))
        assertEquals(ViewerDisplayChoice("off", 2560, 1440), ViewerDisplayChoice.of("off", 2560, 1440))
        assertEquals(ViewerDisplayChoice("request"), ViewerDisplayChoice.of("request", 2560, null))
        assertEquals(ViewerDisplayChoice("auto"), ViewerDisplayChoice.of("later", 0, 1440))
        assertEquals(ViewerDisplayChoice("auto"), ViewerDisplayChoice.of("auto", 16385, 1440))
        // Typed sizes as on the desktop (`parse_size`).
        assertEquals(2560 to 1440, ViewerDisplayChoice.parseSize("2560x1440"))
        assertEquals(1920 to 1080, ViewerDisplayChoice.parseSize(" 1920 × 1080 "))
        assertEquals(1280 to 720, ViewerDisplayChoice.parseSize("1280X720"))
        assertNull(ViewerDisplayChoice.parseSize("1920"))
        assertNull(ViewerDisplayChoice.parseSize("0x1080"))
        assertNull(ViewerDisplayChoice.parseSize("axb"))
        assertEquals("2560×1440", ViewerDisplayChoice("request", 2560, 1440).sizeText)
        assertNull(ViewerDisplayChoice("request").sizeText)
        // The menu lists the desktop viewer's sizes.
        assertEquals(13, ViewerDisplayChoice.SIZES.size)
        assertEquals(listOf("auto", "off", "request"), ViewerDisplayChoice.WISHES)
    }

    @Test
    fun p9_task_android_set_display_names_the_fields_of_view_open() {
        val fixed = ScreenDisplayCalls.args(7, ViewerDisplayChoice("request", 2560, 1440))
        assertEquals("7", fixed["session"].toString())
        assertEquals("\"request\"", fixed["virtualDisplay"].toString())
        assertEquals("2560", fixed["virtualWidth"].toString())
        assertEquals("1440", fixed["virtualHeight"].toString())
        val follow = ScreenDisplayCalls.args(7, ViewerDisplayChoice("off"))
        assertEquals("\"off\"", follow["virtualDisplay"].toString())
        assertNull(follow["virtualWidth"])
        assertNull(follow["virtualHeight"])
    }
}
