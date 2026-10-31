package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * The file rule "neuere ersetzen ältere" on the bridge: the folder preview counts, the transfer
 * result and the line that names the decisions. Decoded with the app's own `Core.json`.
 */
class AbgleichTaskRuleShapesTest {
    @Test
    fun abgleichTaskFolderPreviewDecodesTheFiveCounts() {
        val folder = Core.json.decodeFromString(
            FolderCollision.serializer(),
            """{"source":"/a","relative":"a","newFiles":3,"collidingFiles":10,"newerFiles":4,
               "equalFiles":3,"targetNewerFiles":2,"conflictFiles":1,"counted":true,"complete":true}""",
        )
        assertEquals(3L, folder.newFiles)
        assertEquals(10L, folder.collidingFiles)
        assertEquals(4L, folder.newerFiles)
        assertEquals(3L, folder.equalFiles)
        assertEquals(2L, folder.targetNewerFiles)
        assertEquals(1L, folder.conflictFiles)
    }

    @Test
    fun abgleichTaskOlderBridgeAnswersWithoutTheNewCountsStillDecode() {
        val folder = Core.json.decodeFromString(
            FolderCollision.serializer(),
            """{"source":"/a","relative":"a","newFiles":3,"collidingFiles":10}""",
        )
        assertEquals(0L, folder.newerFiles)
        assertEquals(0L, folder.conflictFiles)
        val result = Core.json.decodeFromString(
            TransferResult.serializer(),
            """{"files":2,"bytes":9,"errors":0,"omitted":0}""",
        )
        assertNull(result.ruleText())
    }

    @Test
    fun abgleichTaskTransferResultNamesTheDecisionsWithoutFileNames() {
        val result = Core.json.decodeFromString(
            TransferResult.serializer(),
            """{"files":9,"bytes":90,"errors":0,"omitted":0,"replaced":4,"skippedEqual":40,
               "keptNewer":2,"keptBoth":1}""",
        )
        assertEquals(
            "4 neuere ersetzt, 40 gleiche übersprungen, 2 Ziel neuer behalten, 1 als Konflikt doppelt behalten",
            result.ruleText(),
        )
    }
}
