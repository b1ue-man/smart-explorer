package app.smartexplorer.android.ui.share

import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.SaverScope
import app.smartexplorer.android.api.ACCESS_ALLOW
import app.smartexplorer.android.api.ACCESS_ASK
import app.smartexplorer.android.api.FOLDERS_SELECTED
import app.smartexplorer.android.api.PRESET_READ_ONLY
import app.smartexplorer.android.api.RIGHTS_FIELD_COMMANDS
import app.smartexplorer.android.api.RIGHTS_FIELD_TERMINAL
import app.smartexplorer.android.api.RIGHTS_FIELD_VALID_UNTIL
import app.smartexplorer.android.api.RIGHTS_FIELD_WRITE
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Plan 2026-10-31 §F, package G3: the draft of the rights sheet survives a configuration change
 * (rotation). The sheet saves it with [RIGHTS_DRAFT_SAVER]; these tests cover the saver without
 * Compose: what is saved comes back as the same draft, and what is not a draft is ignored.
 */
class AbgleichTaskRightsDraftSaverTest {
    private val everything = OfferedRights()
    private val presets = RightsPresets.builtIn()

    private fun rotate(draft: RightsDraft): RightsDraft {
        val saved = with(RIGHTS_DRAFT_SAVER) {
            SaverScope { true }.save(mutableStateOf(draft))
        }
        assertNotNull("the draft is saved", saved)
        return RIGHTS_DRAFT_SAVER.restore(saved!!)!!.value
    }

    @Test
    fun abgleichTaskADraftWithEditsComesBackEqualAfterARotation() {
        val draft = RightsDraft.fromSeed(null, RightsDraft().known, everything)
            .applyPreset(presets.first { it.key == PRESET_READ_ONLY }, everything)
            .setWrite(true)
            .setDecision(RIGHTS_FIELD_COMMANDS, ACCESS_ASK)
            .setExpiry("1d")
        assertEquals(draft, rotate(draft))
    }

    @Test
    fun abgleichTaskASelectionOfFoldersAndAKeptEndSurviveTheRotation() {
        val seed = RightsDraft().rights.copy(validUntil = 1_790_003_600L)
        val draft = RightsDraft.fromSeed(seed, setOf(RIGHTS_FIELD_WRITE, RIGHTS_FIELD_VALID_UNTIL), everything)
            .setFolders(FOLDERS_SELECTED)
            .togglePath("/storage/emulated/0/DCIM")
            .togglePath("/storage/emulated/0/Documents")
        val back = rotate(draft)
        assertEquals(draft, back)
        assertEquals(listOf("/storage/emulated/0/DCIM", "/storage/emulated/0/Documents"), back.rights.folders.paths)
        assertEquals(EXPIRY_KEEP, back.expiry)
        assertEquals(1_790_003_600L, back.keptUntil)
        // Fields the user never touched stay unknown, so they are still not sent.
        assertEquals(draft.known, back.known)
    }

    @Test
    fun abgleichTaskTheRestoredDraftSendsTheSameCallAsTheOneBeforeTheRotation() {
        val draft = RightsDraft().applyPreset(presets.first { it.key == PRESET_READ_ONLY }, everything)
            .setDecision(RIGHTS_FIELD_TERMINAL, ACCESS_ALLOW)
        val now = 1_790_000_000L
        assertEquals(draft.selection(presets, now), rotate(draft).selection(presets, now))
        assertEquals(draft.matchingPreset(presets), rotate(draft).matchingPreset(presets))
    }

    @Test
    fun abgleichTaskSomethingThatIsNotADraftIsIgnoredSoTheSheetStartsFromItsSeed() {
        assertNull(decodeRightsDraft(""))
        assertNull(decodeRightsDraft("not json"))
        assertNull(decodeRightsDraft("""{"rights":"x"}"""))
        assertNull(RIGHTS_DRAFT_SAVER.restore("{"))
        // A draft of a newer app with more fields is still read.
        assertTrue(decodeRightsDraft("""{"expiry":"7d","later":1}""")?.expiry == "7d")
    }
}
