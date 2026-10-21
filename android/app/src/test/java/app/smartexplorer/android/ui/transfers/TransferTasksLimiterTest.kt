package app.smartexplorer.android.ui.transfers

import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.sync.progressText
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test

/** E13: the core's bottleneck text in the transfer bar and in the mirror dialog (U54). */
class TransferTasksLimiterTest {
    private val text = "Begrenzt durch Leitung oder Server (SFTP-Server)"

    private fun task(state: String = "running", limiterText: String? = text) = TaskInfo(
        id = "t1-1",
        kind = "transfer",
        title = "Kopieren",
        state = state,
        doneBytes = 2048,
        totalBytes = 4096,
        doneItems = 1,
        totalItems = 3,
        rateBps = 1024,
        limiter = if (limiterText == null) null else "remote",
        limiterText = limiterText,
    )

    private val base = "1 von 3 Dateien · ${Format.size(2048)} von ${Format.size(4096)} · ${Format.rate(1024)}"

    @Test
    fun `speed_task_android_progress_line_appends_the_limiter_text_while_running`() {
        assertEquals("$base · $text", TransferTasks.progressLine(task()))
        // A message still comes first, the limiter text last.
        assertEquals("Wartet · $base · $text", TransferTasks.progressLine(task().copy(message = "Wartet")))
        // Not shown for a task that is not running, or for a blank text.
        assertFalse(TransferTasks.progressLine(task(state = "done")).contains(text))
        assertNull(TransferTasks.limiterPart(task(state = "done")))
        assertNull(TransferTasks.limiterPart(task(limiterText = "  ")))
    }

    @Test
    fun `speed_task_android_progress_line_without_limiter_is_unchanged`() {
        assertEquals(base, TransferTasks.progressLine(task(limiterText = null)))
        // A core without the keys: the JSON decodes with both fields null.
        val old = Core.json.decodeFromString(
            TaskInfo.serializer(),
            """{"id":"t1-2","kind":"transfer","title":"Kopieren","state":"running","rateBps":1024,"message":null}""",
        )
        assertNull(old.limiter)
        assertNull(old.limiterText)
        assertEquals(Format.rate(1024), TransferTasks.progressLine(old))
        val current = Core.json.decodeFromString(
            TaskInfo.serializer(),
            """{"id":"t1-3","kind":"transfer","title":"Kopieren","state":"running","limiter":"link","limiterText":"Begrenzt durch die Leitung (1 MB/s von 1 MB/s)"}""",
        )
        assertEquals("link", current.limiter)
        assertEquals("Begrenzt durch die Leitung (1 MB/s von 1 MB/s)", current.limiterText)
    }

    @Test
    fun `speed_task_android_mirror_dialog_text_appends_the_limiter_text`() {
        assertEquals("$base · $text", progressText(task()))
        assertEquals(base, progressText(task(limiterText = null)))
        assertEquals("Startet…", progressText(null))
    }
}
