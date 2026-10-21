package app.smartexplorer.android.ui.screen

import app.smartexplorer.android.R
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Every state, error code, end reason and input block of the core has a text of its own in
 * `strings.xml` (SCR-M9, U28-7): `screen_task_android_texts_match_desktop`. The keys come from
 * `src/test/resources/screen_keys.txt`, which the Rust test `screen_task_android_screen_keys_file_current`
 * keeps equal to the core's lists; that Rust file's twin `screen_task_android_strings_match_desktop_sources`
 * compares the words with the desktop viewer.
 */
class ScreenTextsTest {
    private fun keys(): List<Pair<String, String>> {
        val text = javaClass.classLoader!!.getResourceAsStream("screen_keys.txt")!!.bufferedReader().readText()
        return text.lines().filter { it.isNotBlank() }.map { it.substringBefore(':') to it.substringAfter(':') }
    }

    @Test
    fun screen_task_android_texts_match_desktop() {
        val strings = File("src/main/res/values/strings.xml").readText()
        val all = keys()
        assertTrue("the key list is empty", all.size > 30)
        for ((kind, key) in all) {
            val res = when (kind) {
                "state" -> ScreenTexts.stateRes(key)
                "error" -> ScreenTexts.errorRes(key)
                "end" -> ScreenTexts.endRes(key)
                "block" -> ScreenTexts.blockRes(key)
                else -> throw AssertionError("unknown kind $kind")
            }
            assertNotNull("no text for $kind:$key", res)
            val name = "screen_${kind}_$key"
            assertEquals("$name points elsewhere", R.string::class.java.getField(name).getInt(null), res)
            assertTrue("$name is missing in strings.xml", strings.contains("<string name=\"$name\">"))
        }
        // A key of a newer host has no text of its own.
        assertEquals(null, ScreenTexts.errorRes("from_the_future"))
    }

    @Test
    fun a_detail_follows_only_where_the_desktop_shows_one() {
        assertEquals("Die Sitzung ist fehlgeschlagen: kaputt", ScreenTexts.withDetail("Die Sitzung ist fehlgeschlagen", " kaputt "))
        assertEquals("Die Sitzung ist fehlgeschlagen", ScreenTexts.withDetail("Die Sitzung ist fehlgeschlagen", "  "))
        assertEquals("Die Sitzung ist fehlgeschlagen", ScreenTexts.withDetail("Die Sitzung ist fehlgeschlagen", null))
    }
}
