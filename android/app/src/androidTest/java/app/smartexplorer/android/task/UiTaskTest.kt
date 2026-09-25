package app.smartexplorer.android.task

import android.graphics.Bitmap
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isSelectable
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.smartexplorer.android.MainActivity
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreState
import app.smartexplorer.android.prefs.AppPrefs
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * G6 layout check of the real MainActivity: the four tabs, the places side panel, the filter row
 * and the Sync, Share and More pages, found by their German labels and content descriptions
 * (the only test tag is the places list, to scroll it). Screenshots go to
 * `<filesDir>/task-report/screens/` for the artifact; they are a layout record, not a pixel
 * comparison.
 */
@RunWith(AndroidJUnit4::class)
class UiTaskTest {
    @get:Rule
    val compose = createAndroidComposeRule<MainActivity>()

    @Before
    fun skipOnboarding() {
        AppPrefs.setOnboardingDone(true)
    }

    private fun waitForNode(matcher: SemanticsMatcher, what: String, timeoutMs: Long = 30_000) {
        try {
            // Merged tree for clickable containers (tabs, icon buttons), unmerged for inner texts.
            compose.waitUntil(timeoutMillis = timeoutMs) {
                compose.onAllNodes(matcher).fetchSemanticsNodes().isNotEmpty() ||
                    compose.onAllNodes(matcher, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
            }
        } catch (e: Throwable) {
            screenshot("fehler-" + what.replace(Regex("[^A-Za-z0-9]+"), "-"))
            throw AssertionError("Element fehlt nach ${timeoutMs / 1000} s: $what", e)
        }
    }

    private fun tab(label: String): SemanticsMatcher = hasText(label) and isSelectable()

    private fun screenshot(name: String) {
        compose.waitForIdle()
        val bitmap = compose.onRoot().captureToImage().asAndroidBitmap()
        TaskReport.file("screens/$name.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
    }

    @Test
    fun mainViewsTabsSidePanelAndFilterRow() {
        compose.waitUntil(timeoutMillis = 120_000) { Core.ready.value == CoreState.Ready }
        for (label in listOf("Dateien", "Sync", "Teilen", "Mehr")) waitForNode(tab(label), "Tab $label")
        waitForNode(hasContentDescription("Orte"), "Knopf Orte (Seitenleiste)")
        waitForNode(hasContentDescription("Filtern"), "Knopf Filtern")
        screenshot("01-dateien")

        compose.onNode(hasContentDescription("Filtern") and hasClickAction()).performClick()
        waitForNode(hasText("Rekursiv"), "Filterzeile: Rekursiv")
        waitForNode(hasText("Name…"), "Filterzeile: Namensfeld")
        waitForNode(hasContentDescription("Filter"), "Filterzeile: Filter-Blatt")
        screenshot("02-filterzeile")
        compose.onNode(hasContentDescription("Filter schließen") and hasClickAction()).performClick()

        compose.onNode(tab("Sync")).performClick()
        waitForNode(hasContentDescription("Job anlegen"), "Sync: Job anlegen")
        waitForNode(hasContentDescription("Aktualisieren"), "Sync: Aktualisieren")
        screenshot("03-sync")

        compose.onNode(tab("Teilen")).performClick()
        waitForNode(hasText("Raum erstellen"), "Teilen: Raum erstellen")
        waitForNode(hasText("Raum beitreten"), "Teilen: Raum beitreten")
        waitForNode(hasText("Gerät verbinden"), "Teilen: Gerät verbinden")
        screenshot("04-teilen")

        compose.onNode(tab("Mehr")).performClick()
        for (entry in listOf("Speicheranalyse", "Duplikate finden", "Verbindungen", "Papierkorb", "Einstellungen", "Fehlerprotokoll", "Über")) {
            waitForNode(hasText(entry), "Mehr: $entry")
        }
        screenshot("05-mehr")

        compose.onNode(tab("Dateien")).performClick()
        waitForNode(hasContentDescription("Orte"), "Dateien nach Tabwechsel")
        compose.onNode(hasContentDescription("Orte") and hasClickAction()).performClick()
        waitForNode(hasText("Ordner suchen"), "Seitenleiste: Ordner suchen")
        screenshot("06-seitenleiste")
        // Up to ten recent places push the lower sections below the screen: scroll the list.
        val places = compose.onNode(hasTestTag("places-list"))
        places.performScrollToNode(hasText("Verbindung hinzufügen"))
        waitForNode(hasText("Verbindungen"), "Seitenleiste: Verbindungen")
        places.performScrollToNode(hasText("Google Drive"))
        waitForNode(hasText("Google Drive"), "Seitenleiste: Google Drive")
        screenshot("07-seitenleiste-unten")
    }
}
