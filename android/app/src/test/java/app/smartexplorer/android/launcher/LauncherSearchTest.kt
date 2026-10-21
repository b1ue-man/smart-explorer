package app.smartexplorer.android.launcher

import app.smartexplorer.android.launcher.search.LauncherSearch
import java.text.Collator
import java.util.Locale
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** The matching of the app search (U35-25), without Android. */
class LauncherSearchTest {
    private val collator = Collator.getInstance(Locale.ROOT).apply { strength = Collator.PRIMARY }

    private fun search(query: String, vararg labels: String) = LauncherSearch.filter(labels.toList(), query, { it }, collator)

    @Test
    fun accentsAndCaseAreIgnored() {
        assertEquals("cafe uber arger", LauncherSearch.normalize("Café ÜBER Ärger"))
        assertEquals(listOf("Übersicht"), search("uber", "Übersicht", "Kamera"))
        assertEquals(listOf("Café"), search("CAFE", "Café", "Kamera"))
        assertEquals(listOf("cafe"), LauncherSearch.tokens("  Café "))
    }

    @Test
    fun theStartOfAWordRanksBeforeAPartOfAWord() {
        assertEquals(0, LauncherSearch.rank(listOf("cam"), "Alpha Camera"))
        assertEquals(0, LauncherSearch.rank(listOf("cam"), "Camera Pro"))
        assertEquals(1, LauncherSearch.rank(listOf("cam"), "Webcam"))
        assertNull(LauncherSearch.rank(listOf("cam"), "Browser"))
        assertEquals(listOf("Alpha Camera", "Camera Pro", "Webcam"), search("cam", "Webcam", "Camera Pro", "Alpha Camera", "Browser"))
    }

    @Test
    fun everyWordOfTheQueryMustMatch() {
        assertEquals(0, LauncherSearch.rank(listOf("pro", "cam"), "Camera Pro"))
        assertEquals(1, LauncherSearch.rank(listOf("pro", "cam"), "Webcam Pro"))
        assertNull(LauncherSearch.rank(listOf("pro", "cam"), "Camera"))
        assertEquals(listOf("Camera Pro", "Webcam Pro"), search("pro cam", "Webcam Pro", "Camera Pro", "Camera"))
        // Words of the name are split at everything but letters and digits.
        assertEquals(0, LauncherSearch.rank(listOf("mail"), "e-mail (work)"))
    }

    @Test
    fun anEmptyQueryShowsTheWholeListInItsOrder() {
        assertEquals(listOf("b", "a"), search("", "b", "a"))
        assertEquals(listOf("b", "a"), search("   ", "b", "a"))
        assertEquals(emptyList<String>(), search("zzz", "b", "a"))
    }
}
