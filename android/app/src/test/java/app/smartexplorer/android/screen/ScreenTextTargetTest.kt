package app.smartexplorer.android.screen

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** [hidesContent]: typed text never replaces content the accessibility service cannot see (round 12, S7 / Codex 12/1). */
class ScreenTextTargetTest {
    @Test
    fun screen_task_android_text_never_replaces_hidden_content() {
        val edit = "android.widget.EditText"
        // An ordinary field with or without text, and one showing its hint, take text.
        assertFalse(hidesContent(false, "abc", false, 3, edit))
        assertFalse(hidesContent(false, "", false, -1, edit))
        assertFalse(hidesContent(false, "Name", true, -1, edit))
        // A web or custom field that reports no text, no hint and no cursor says nothing about its content.
        assertTrue(hidesContent(false, null, false, -1, "android.view.View"))
        assertTrue(hidesContent(false, null, false, -1, edit))
        // A password field: empty (no text, no cursor) in the platform's family, with content masked.
        assertFalse(hidesContent(true, null, false, -1, edit))
        assertFalse(hidesContent(true, "Passwort", true, -1, edit))
        assertTrue(hidesContent(true, "•••", false, 3, edit))
        // A password field of a web or custom view without a cursor is no proof of an empty field.
        assertTrue(hidesContent(true, null, false, -1, "android.view.View"))
        assertTrue(hidesContent(true, null, false, -1, null))
    }
}
