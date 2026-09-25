package app.smartexplorer.android.task

import androidx.test.ext.junit.runners.AndroidJUnit4
import app.smartexplorer.android.core.Entry
import app.smartexplorer.android.core.VolumeInfo
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * G4 evidence for the Android no-replace rename chain (`android_fs::rename_no_replace`):
 * renames, trash moves and restores on one volume, then the lines the core appended to
 * `<filesDir>/smart_explorer/android-fs.log`. The core logs each (attempt, errno) once per
 * process, so the host script runs each method in its own `am instrument` process.
 */
@RunWith(AndroidJUnit4::class)
class RenameProbeTaskTest {
    @Test
    fun internalVolume() = probe(Volumes.primary(), "internal")

    @Test
    fun sdCardVolume() = probe(Volumes.sdCard(), "sdcard")

    private fun probe(volume: VolumeInfo, label: String) = coreTest {
        val log = File(appContext.filesDir, "smart_explorer/android-fs.log")
        val before = if (log.isFile) log.readLines().size else 0
        val dir = Fixture.dir(volume, "rename-probe").absolutePath
        val file = Fixture.write(File(dir, "datei.txt"), "r")
        val folder = File(dir, "ordner").apply { mkdirs() }
        Fixture.write(File(folder, "kind.txt"), "k")

        val renamedFile: Entry = Api.get("fs.rename", args("location" to file.absolutePath, "newName" to "datei2.txt"))
        val renamedFolder: Entry = Api.get("fs.rename", args("location" to folder.absolutePath, "newName" to "ordner2"))
        assertTrue(File(renamedFile.location).isFile)
        assertTrue(File(renamedFolder.location, "kind.txt").isFile)
        // Never replace: renaming onto an existing name fails and keeps both.
        Fixture.write(File(dir, "belegt.txt"), "b")
        Api.failure("fs.rename", args("location" to renamedFile.location, "newName" to "belegt.txt"), "exists")
        assertEquals("b", File(dir, "belegt.txt").readText())

        Api.runTask("fs.delete", args("locations" to listOf(renamedFile.location, renamedFolder.location), "permanent" to false))
        assertFalse(File(renamedFile.location).exists())
        val ids = Api.objects("trash.list")
            .filter { it.text("originalLocation") in setOf(renamedFile.location, renamedFolder.location) }
            .map { it.text("id") }
        assertEquals(2, ids.size)
        assertEquals(2, Api.obj("trash.restore", args("ids" to ids)).int("restored"))
        assertTrue(File(renamedFolder.location, "kind.txt").isFile)

        val lines = if (log.isFile) log.readLines().drop(before) else emptyList()
        val text = buildString {
            append("volume=").append(volume.path).append(" label=").append(label).append('\n')
            if (lines.isEmpty()) append("keine Ausweichschritte protokolliert (renameat2 RENAME_NOREPLACE wirkte direkt)\n")
            lines.forEach { append(it).append('\n') }
        }
        TaskReport.file("rename-errno-$label.txt").writeText(text)
        TaskReport.note("rename-errno", text.trim().replace('\n', ' '))
    }
}
