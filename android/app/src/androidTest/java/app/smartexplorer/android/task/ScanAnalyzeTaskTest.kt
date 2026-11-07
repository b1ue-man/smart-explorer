package app.smartexplorer.android.task

import androidx.test.ext.junit.runners.AndroidJUnit4
import app.smartexplorer.android.api.AnalyzeNode
import app.smartexplorer.android.api.DuplicateGroup
import java.io.File
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/** G4: folder index, storage analysis and duplicates. */
@RunWith(AndroidJUnit4::class)
class ScanAnalyzeTaskTest {
    @Test
    fun folderIndexFindsANewFolder() = coreTest {
        val marker = Fixture.unique("seindexmarker")
        val dir = File(Fixture.dir(Volumes.primary(), "index"), marker).apply { mkdirs() }
        Api.obj("index.status")
        Api.runTask("index.build", timeoutMs = 600_000)
        val status = Api.obj("index.status")
        assertEquals("ready", status.text("state"))
        assertTrue(status.int("count") > 0)
        val hits = Api.objects("index.search", args("query" to marker, "limit" to 10))
        assertTrue("Ordnersuche fand $marker nicht: $hits", hits.any { it.text("location") == dir.absolutePath && it.text("name") == marker })
    }

    @Test
    fun storageAnalysisAndDuplicates() = coreTest {
        val root = Fixture.dir(Volumes.primary(), "analyse")
        Fixture.bytes(File(root, "gross.bin"), 300_000, 1)
        Fixture.bytes(File(root, "sub/mittel.bin"), 200_000, 2)
        Fixture.bytes(File(root, "sub/klein.bin"), 10_000, 3)
        val dup1 = Fixture.bytes(File(root, "kopie1.dat"), 50_000, 9)
        val dup2 = Fixture.bytes(File(root, "sub/kopie2.dat"), 50_000, 9)

        val analysis = Api.runTask("analyze.start", args("location" to root.absolutePath))
        val top: AnalyzeNode = Api.get("analyze.node", args("taskId" to analysis.id, "path" to emptyList<String>()))
        assertEquals(listOf("gross.bin", "sub", "kopie1.dat"), top.children.map { it.name })
        assertEquals(top.children.sortedByDescending { it.size }, top.children)
        assertEquals(root.absolutePath, top.location)
        val sub: AnalyzeNode = Api.get("analyze.node", args("taskId" to analysis.id, "path" to listOf("sub")))
        assertEquals("mittel.bin", sub.children.first().name)
        assertEquals(260_000L, sub.size)
        assertTrue(Api.obj("analyze.issues", args("taskId" to analysis.id)).int("count") >= 0)

        val reclaim = Api.runTask("reclaim.start", args("location" to root.absolutePath, "minSize" to 1024))
        val groups: List<DuplicateGroup> = Api.get("reclaim.groups", args("taskId" to reclaim.id))
        val group = groups.firstOrNull { g -> g.items.map { it.location }.toSet() == setOf(dup1.absolutePath, dup2.absolutePath) }
        assertNotNull("Duplikatgruppe fehlt: $groups", group)
        assertEquals(50_000L, group!!.size)
        // Deleting a chosen copy goes to the app trash.
        Api.runTask("fs.delete", args("locations" to listOf(dup2.absolutePath), "permanent" to false))
        assertFalse(dup2.exists())
        assertTrue(dup1.exists())
        val result: JsonObject = analysis.resultObj()
        assertTrue(result.long("files") >= 5)
    }
}
