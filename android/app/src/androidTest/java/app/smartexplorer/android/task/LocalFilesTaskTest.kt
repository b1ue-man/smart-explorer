package app.smartexplorer.android.task

import androidx.test.ext.junit.runners.AndroidJUnit4
import app.smartexplorer.android.api.Listing
import app.smartexplorer.android.core.Entry
import java.io.File
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * G4 local file flows on both volumes: create, rename, check, list with filter, copy with every
 * conflict choice, move between volumes, filtered relative copy, properties, app trash per volume
 * with restore, ZIP browsing/fetch/extract and the protected trash omission in local copies.
 */
@RunWith(AndroidJUnit4::class)
class LocalFilesTaskTest {
    @Test
    fun createRenameCheckAndFilteredListing() = coreTest {
        val dir = Fixture.dir(Volumes.primary(), "local", "basics").absolutePath
        val folder: Entry = Api.mkdir(dir, "Ordner")
        assertTrue(folder.isDir)
        val file: Entry = Api.get("fs.newFile", args("parent" to dir, "name" to "notiz.txt"))
        assertFalse(file.isDir)
        Fixture.write(File(dir, "alpha.log"), "0123456789")
        Fixture.write(File(dir, ".versteckt"), "x")

        val existing = Api.obj("fs.checkName", args("parent" to dir, "name" to "notiz.txt"))
        assertTrue(existing.bool("exists"))
        val hostile = Api.obj("fs.checkName", args("parent" to dir, "name" to "CON"))
        assertFalse(hostile.bool("exists"))
        assertNotNull("CON ist unter Windows reserviert", hostile.textOrNull("problem"))

        val renamed: Entry = Api.get("fs.rename", args("location" to file.location, "newName" to "umbenannt.txt"))
        assertEquals("umbenannt.txt", renamed.name)
        assertFalse(File(file.location).exists())
        Api.failure("fs.rename", args("location" to renamed.location, "newName" to "alpha.log"), "exists")
        val stat: Entry = Api.get("fs.stat", args("location" to File(dir, "alpha.log").absolutePath))
        assertEquals(10L, stat.size)
        assertEquals("log", stat.ext.trimStart('.'))

        val visible = Api.listing(dir, showHidden = false)
        assertEquals(setOf("Ordner", "umbenannt.txt", "alpha.log"), visible.entries.map { it.name }.toSet())
        assertEquals("local", visible.backend)
        assertTrue(visible.canTrash)
        assertEquals(dir, visible.crumbs.last().location)
        assertTrue(".versteckt" in Api.names(dir))
        val filtered = Api.listing(dir, showHidden = false, filter = mapOf("text" to "alpha", "mode" to "substring", "files" to true, "dirs" to false))
        assertEquals(listOf("alpha.log"), filtered.entries.map { it.name })
        val bySize = Api.listing(dir, showHidden = false, sortKey = "size")
        assertEquals("Ordner", bySize.entries.first().name)
    }

    @Test
    fun copyConflictsMoveBetweenVolumesAndFilteredCopy() = coreTest {
        val source = Fixture.dir(Volumes.primary(), "local", "copy-src")
        val target = Fixture.dir(Volumes.primary(), "local", "copy-dst")
        val a = Fixture.write(File(source, "a.txt"), "neu")
        Fixture.write(File(target, "a.txt"), "alt")

        val conflicts = Api.obj("fs.conflicts", args("sources" to listOf(a.absolutePath), "targetDir" to target.absolutePath))
        assertEquals(listOf("a.txt"), conflicts.texts("names"))
        assertTrue(conflicts.bool("choosable"))

        Api.copy(listOf(a.absolutePath), target.absolutePath, conflict = "skip")
        assertEquals("alt", File(target, "a.txt").readText())
        Api.copy(listOf(a.absolutePath), target.absolutePath, conflict = "keepBoth")
        assertEquals("neu", File(target, "a (2).txt").readText())
        assertEquals("alt", File(target, "a.txt").readText())
        Api.copy(listOf(a.absolutePath), target.absolutePath, conflict = "replace")
        assertEquals("neu", File(target, "a.txt").readText())

        // Move from the internal volume to the SD card (different file systems).
        val sdTarget = Fixture.dir(Volumes.sdCard(), "local", "move-dst")
        val moving = Fixture.bytes(File(source, "wandert.bin"), 256 * 1024, 7)
        val hash = Fixture.sha256(moving)
        Api.runTask(
            "fs.transfer",
            args("sources" to listOf(moving.absolutePath), "targetDir" to sdTarget.absolutePath, "mode" to "move", "conflict" to "keepBoth"),
        )
        assertFalse("Quelle nach dem Verschieben noch da", moving.exists())
        assertEquals(hash, Fixture.sha256(File(sdTarget, "wandert.bin")))

        // Filter + baseDir: only matching files, paths relative to baseDir (desktop rule).
        val base = Fixture.dir(Volumes.primary(), "local", "filter-base")
        Fixture.write(File(base, "sub/eins.txt"), "1")
        Fixture.write(File(base, "sub/zwei.bin"), "2")
        Fixture.write(File(base, "sub/tief/drei.txt"), "3")
        val filteredTarget = Fixture.dir(Volumes.primary(), "local", "filter-dst")
        Api.runTask(
            "fs.transfer",
            args(
                "sources" to listOf(File(base, "sub").absolutePath),
                "targetDir" to filteredTarget.absolutePath,
                "mode" to "copy",
                "conflict" to "keepBoth",
                "filter" to mapOf("text" to "*.txt", "mode" to "glob", "files" to true, "dirs" to false),
                "baseDir" to base.absolutePath,
            ),
        )
        assertTrue(File(filteredTarget, "sub/eins.txt").isFile)
        assertTrue(File(filteredTarget, "sub/tief/drei.txt").isFile)
        assertFalse(File(filteredTarget, "sub/zwei.bin").exists())
    }

    @Test
    fun propertiesAndPermanentDelete() = coreTest {
        val dir = Fixture.dir(Volumes.primary(), "local", "props")
        Fixture.bytes(File(dir, "x/1.bin"), 1000, 1)
        Fixture.bytes(File(dir, "x/y/2.bin"), 2000, 2)
        val properties = Api.runTask("fs.properties", args("locations" to listOf(File(dir, "x").absolutePath))).resultObj()
        assertEquals(2L, properties.long("files"))
        assertEquals(1L, properties.long("dirs"))
        assertEquals(3000L, properties.long("bytes"))
        assertEquals(File(dir, "x").absolutePath, properties.text("location"))
        val deleted = Api.runTask("fs.delete", args("locations" to listOf(File(dir, "x").absolutePath), "permanent" to true))
        assertFalse(File(dir, "x").exists())
        assertTrue(deleted.resultObj().long("deleted") >= 1)
        // Places without an app trash (the app's private storage) answer `unsupported`.
        val privateFile = Fixture.write(File(appContext.filesDir, "task-private.txt"), "p")
        Api.failure("fs.delete", args("locations" to listOf(privateFile.absolutePath), "permanent" to false), "unsupported")
    }

    @Test
    fun appTrashPerVolumeRestoreDeleteAndEmpty() = coreTest {
        val originals = listOf(Volumes.primary(), Volumes.sdCard()).map { volume ->
            val dir = Fixture.dir(volume, "local", "trash")
            val file = Fixture.write(File(dir, "weg.txt"), "Papierkorb ${volume.path}")
            val folder = File(dir, "Ordner").apply { mkdirs() }
            Fixture.write(File(folder, "inhalt.txt"), "i")
            Api.runTask("fs.delete", args("locations" to listOf(file.absolutePath, folder.absolutePath), "permanent" to false))
            assertFalse(file.exists())
            assertFalse(folder.exists())
            val trash = Volumes.trashDir(volume)
            assertTrue("Papierkorb fehlt auf ${volume.path}", trash.isDirectory)
            Triple(volume, file, folder)
        }
        val items = Api.objects("trash.list")
        for ((volume, file, folder) in originals) {
            assertTrue("${file.path} fehlt im Papierkorb", items.any { it.text("originalLocation") == file.absolutePath && !it.bool("isDir") })
            assertTrue("${folder.path} fehlt im Papierkorb", items.any { it.text("originalLocation") == folder.absolutePath && it.bool("isDir") })
            // The trash of each volume stays on that volume (rename, never a cross-volume copy).
            assertTrue(Volumes.trashDir(volume).walkTopDown().any { it.name == "weg.txt" })
        }
        val trashListing: Listing = Api.get(
            "fs.list",
            args("location" to "trash://", "showHidden" to true, "filter" to null, "sort" to mapOf("key" to "name", "desc" to false, "dirsFirst" to true)),
        )
        assertEquals("trash", trashListing.backend)
        assertTrue(trashListing.entries.any { it.name == "weg.txt" })

        // Restore the internal file while its name is taken again → "weg (2).txt".
        val (_, internalFile, internalFolder) = originals.first()
        Fixture.write(internalFile, "neu belegt")
        val restoreIds = items.filter { it.text("originalLocation") in setOf(internalFile.absolutePath, internalFolder.absolutePath) }.map { it.text("id") }
        val restored = Api.obj("trash.restore", args("ids" to restoreIds))
        assertEquals(2, restored.int("restored"))
        assertEquals(1, restored.int("renamed"))
        assertEquals("Papierkorb ${Volumes.primary().path}", File(internalFile.parentFile, "weg (2).txt").readText())
        assertTrue(File(internalFolder, "inhalt.txt").isFile)

        val (_, sdFile, sdFolder) = originals.last()
        val sdFileId = items.first { it.text("originalLocation") == sdFile.absolutePath }.text("id")
        Api.runTask("trash.delete", args("ids" to listOf(sdFileId)))
        assertFalse(Api.objects("trash.list").any { it.text("id") == sdFileId })
        assertTrue(Api.objects("trash.list").any { it.text("originalLocation") == sdFolder.absolutePath })
        Api.runTask("trash.empty")
        assertTrue(Api.objects("trash.list").isEmpty())
        assertTrue(Api.obj("trash.purge", args("olderThanDays" to 30)).int("removed") >= 0)
    }

    @Test
    fun zipBrowseFetchAndExtract() = coreTest {
        val dir = Fixture.dir(Volumes.primary(), "local", "zip")
        val archive = File(dir, "archiv.zip")
        ZipOutputStream(archive.outputStream()).use { zip ->
            zip.putNextEntry(ZipEntry("innen/a.txt"))
            zip.write("Inhalt A".toByteArray())
            zip.closeEntry()
            zip.putNextEntry(ZipEntry("b.txt"))
            zip.write("Inhalt B".toByteArray())
            zip.closeEntry()
        }
        val root = Api.listing(archive.absolutePath)
        assertEquals("zip", root.backend)
        assertTrue(root.readOnly)
        assertEquals(dir.absolutePath, root.parent)
        val inner = root.entries.first { it.name == "innen" }
        val a = Api.listing(inner.location).entries.first { it.name == "a.txt" }
        assertTrue(a.location.startsWith("zip://"))
        Api.failure("fs.open", args("location" to a.location), "unsupported")
        val fetched = Api.runTask("fs.fetch", args("location" to a.location)).resultObj()
        assertEquals("Inhalt A", File(fetched.text("localPath")).readText())
        Api.failure("fs.mkdir", args("parent" to root.location, "name" to "neu"), "permission")

        val extracted = Api.runTask("fs.extract", args("location" to archive.absolutePath, "targetDir" to null)).resultObj()
        val folder = File(extracted.text("location"))
        assertEquals(dir.absolutePath, folder.parent)
        assertEquals("Inhalt A", File(folder, "innen/a.txt").readText())
        assertEquals("Inhalt B", File(folder, "b.txt").readText())
        assertEquals(2L, extracted.long("files"))
        Api.call("fs.discardEdit", args("editId" to fetched.text("editId")))
    }

    @Test
    fun trashFolderNamesAreOmittedFromLocalCopies() = coreTest {
        val source = Fixture.dir(Volumes.primary(), "local", "omit-src")
        Fixture.write(File(source, "behalten.txt"), "k")
        Fixture.write(File(source, ".SmartExplorer-Papierkorb/geheim.txt"), "t")
        val target = Fixture.dir(Volumes.sdCard(), "local", "omit-dst")
        Api.copy(listOf(source.absolutePath), target.absolutePath)
        assertTrue(File(target, "omit-src/behalten.txt").isFile)
        assertFalse("Papierkorb-Ordner wurde mitkopiert", File(target, "omit-src/.SmartExplorer-Papierkorb").exists())
        assertNull(Api.listing(File(target, "omit-src").absolutePath).entries.firstOrNull { it.name == ".SmartExplorer-Papierkorb" })
    }
}
