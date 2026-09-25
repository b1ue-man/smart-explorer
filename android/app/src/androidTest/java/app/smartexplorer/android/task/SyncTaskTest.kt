package app.smartexplorer.android.task

import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.File
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * G4 sync: internal storage ↔ SD-card job with validation, run, conflicts (check, resolve, merge,
 * keep both, skip, finish; plain SFTP cannot replace files, so conflicts run between two local
 * volumes), a job from the SD-card root to SFTP with deletions in both directions (app trash, the
 * trash folder never uploaded, other apps' private folders omitted) and mirroring the SD-card
 * root (trash folder omitted).
 * Every job-bound call waits for its task: one task per job at a time (busy otherwise).
 */
@RunWith(AndroidJUnit4::class)
class SyncTaskTest {
    private val original = "Zeile 1\nZeile 2\nZeile 3\n"

    private suspend fun conflicts(jobId: String): List<JsonObject> {
        val answer = Api.obj("sync.conflicts", args("id" to jobId))
        assertTrue("Konfliktkontext fehlt: $answer", answer.bool("available"))
        return answer.objects("items")
    }

    private fun cidOf(items: List<JsonObject>, fileName: String): JsonPrimitive {
        val item = items.firstOrNull { it.text("path").substringAfterLast('/') == fileName }
            ?: throw AssertionError("Konflikt für $fileName fehlt: $items")
        return item.field("cid") as? JsonPrimitive ?: throw AssertionError("cid ist kein Wert: $item")
    }

    @Test
    fun localConflictsResolveMergeKeepBothAndSkip() = coreTest(timeoutMs = 25 * 60_000L) {
        val local = Fixture.dir(Volumes.primary(), "sync", "konflikt")
        val other = Fixture.dir(Volumes.sdCard(), "sync", "konflikt-b")
        val remote = other.absolutePath
        val invalid = Api.obj("sync.validate", args("job" to SyncJobs.job("id" to "", "source" to "", "target" to remote)))
        assertTrue("source-Fehler fehlt: $invalid", "source" in invalid.field("errors").obj())
        val draft = SyncJobs.job(
            "id" to "",
            "name" to "Task Konflikte",
            "source" to local.absolutePath,
            "target" to remote,
            "direction" to "both",
            "conflict" to "strict",
            "deletePolicy" to "propagate",
            "trigger" to "manual",
            "enabled" to true,
        )
        assertTrue(Api.obj("sync.validate", args("job" to draft)).field("errors").obj().isEmpty())
        val job = Api.obj("sync.save", args("job" to draft))
        val id = job.text("id")
        assertTrue(id.isNotEmpty())
        assertFalse(Api.obj("sync.setEnabled", args("id" to id, "enabled" to false)).bool("enabled"))
        assertTrue(Api.obj("sync.setEnabled", args("id" to id, "enabled" to true)).bool("enabled"))

        val names = listOf("k1.txt", "k2.txt", "k3.txt", "k4.txt")
        names.forEach { Fixture.write(File(local, it), original) }
        Fixture.write(File(local, "nur-a.txt"), "a")
        val first = Api.runTask("sync.run", args("id" to id)).resultObj()
        assertEquals(0, first.int("errors"))
        assertEquals(0, first.int("conflicts"))
        assertTrue(first.int("aToB") >= 5)
        assertTrue(Api.names(remote).containsAll(names + "nur-a.txt"))
        assertNotNull(SyncJobs.byId(id).objectOrNullField("lastResult"))

        // The same file changes on both sides → strict conflicts.
        names.forEach { Fixture.write(File(local, it), "Zeile 1\nA\nZeile 3\n") }
        names.forEach { Fixture.write(File(other, it), "Zeile 1\nB\nZeile 3\n") }
        val second = Api.runTask("sync.run", args("id" to id)).resultObj()
        assertEquals(4, second.int("conflicts"))
        assertEquals(4, conflicts(id).size)
        val check = Api.runTask("sync.checkConflicts", args("id" to id)).resultObj()
        assertEquals(4, check.int("conflicts"))
        var items = conflicts(id)

        val resolved = Api.runTask("sync.resolve", args("id" to id, "cid" to cidOf(items, "k1.txt"), "choice" to "a")).resultObj()
        assertEquals(3, resolved.int("remaining"))
        assertEquals("Zeile 1\nA\nZeile 3\n", File(other, "k1.txt").readText())

        items = conflicts(id)
        val k2 = cidOf(items, "k2.txt")
        val rows = Api.obj("sync.mergeRows", args("id" to id, "cid" to k2)).objects("rows")
        assertTrue(rows.isNotEmpty())
        // Keep every line of both sides (a changed line appears once per side).
        val choices = rows.map { row -> mapOf("takeA" to (row.textOrNull("a") != null), "takeB" to (row.textOrNull("b") != null)) }
        Api.runTask("sync.mergeApply", args("id" to id, "cid" to k2, "rows" to choices))
        val merged = File(local, "k2.txt").readText()
        assertTrue("Zusammenführung ohne A/B: $merged", "A" in merged && "B" in merged)
        assertEquals(merged, File(other, "k2.txt").readText())

        items = conflicts(id)
        Api.runTask("sync.mergeKeepBoth", args("id" to id, "cid" to cidOf(items, "k3.txt")))
        assertEquals("Zeile 1\nA\nZeile 3\n", File(local, "k3.txt").readText())
        val keptLocal = local.list().orEmpty().filter { it.startsWith("k3") && it.contains("Konflikt") }
        assertEquals("Konfliktkopie lokal: ${local.list()?.toList()}", 1, keptLocal.size)
        assertTrue(keptLocal.single() in Api.names(remote))

        items = conflicts(id)
        Api.call("sync.skip", args("id" to id, "cid" to cidOf(items, "k4.txt")))
        Api.call("sync.finishConflicts", args("id" to id))
        Api.runTask("sync.run", args("id" to id))
        Api.call("sync.delete", args("id" to id))
        assertFalse(Api.objects("sync.jobs").any { it.text("id") == id })
    }

    @Test
    fun sdCardRootJobPropagatesDeletionsThroughTheAppTrash() = coreTest(timeoutMs = 25 * 60_000L) {
        val sd = Volumes.sdCard()
        val keep = File(sd.path, "se-task-root-keep.txt")
        val drop = File(sd.path, "se-task-root-drop.txt")
        val remoteDrop = File(sd.path, "se-task-root-remote-drop.txt")
        listOf(keep, drop, remoteDrop).forEach { Fixture.write(it, it.name) }
        val remote = Servers.freshDir(Servers.sftp(), "sdroot")
        val job = SyncJobs.save(
            "name" to "Task SD-Wurzel",
            "source" to sd.path,
            "target" to remote,
            "direction" to "both",
            "conflict" to "newer",
            "deletePolicy" to "propagate",
            "trigger" to "manual",
            "useRecycleBin" to true,
            "enabled" to true,
        )
        val id = job.text("id")
        val first = Api.runTask("sync.run", args("id" to id), timeoutMs = 600_000).resultObj()
        TaskReport.note("sd-root-job", "erster Lauf: ${first.textOrNull("summary")}")
        val uploaded = Api.names(remote)
        assertTrue(uploaded.containsAll(listOf(keep.name, drop.name, remoteDrop.name)))
        assertFalse("App-Papierkorb wurde hochgeladen", ".SmartExplorer-Papierkorb" in uploaded)

        Api.runTask("fs.delete", args("locations" to listOf(drop.absolutePath), "permanent" to false))
        Api.runTask("fs.delete", args("locations" to listOf(Api.child(remote, remoteDrop.name).location), "permanent" to true))
        val second = Api.runTask("sync.run", args("id" to id), timeoutMs = 600_000).resultObj()
        TaskReport.note("sd-root-job", "zweiter Lauf: ${second.textOrNull("summary")}")
        assertTrue("Löschungen nicht übernommen: $second", second.int("deleted") >= 2)
        val after = Api.names(remote)
        assertFalse(drop.name in after)
        assertTrue(keep.name in after)
        assertFalse("App-Papierkorb wurde hochgeladen", ".SmartExplorer-Papierkorb" in after)
        assertFalse(remoteDrop.exists())
        // The deletion from the remote side went to the SD card's app trash, not away.
        val trash = Api.objects("trash.list")
        assertTrue("Remote-Löschung nicht im App-Papierkorb: $trash", trash.any { it.text("originalLocation") == remoteDrop.absolutePath })
        assertTrue(trash.any { it.text("originalLocation") == drop.absolutePath })
        Api.call("sync.delete", args("id" to id))
        keep.delete()
    }

    @Test
    fun mirroringTheSdCardRootOmitsTheAppTrash() = coreTest(timeoutMs = 25 * 60_000L) {
        val sd = Volumes.sdCard()
        val probeDir = Fixture.dir(sd, "mirror")
        val probe = Fixture.write(File(probeDir, "in-den-papierkorb.txt"), "t")
        Fixture.write(File(probeDir, "bleibt.txt"), "b")
        Api.runTask("fs.delete", args("locations" to listOf(probe.absolutePath), "permanent" to false))
        assertTrue(Volumes.trashDir(sd).isDirectory)
        val target = Fixture.dir(Volumes.primary(), "sync", "mirror-sd")
        val mirror = Api.runTask("sync.mirror", args("source" to sd.path, "target" to target.absolutePath), timeoutMs = 600_000).resultObj()
        TaskReport.note("mirror", mirror.toString())
        assertTrue(mirror.long("copied") >= 1)
        assertNotNull("Auslassung des Papierkorbs nicht gemeldet: $mirror", mirror.textOrNull("omitted"))
        assertTrue(File(target, "${Fixture.FOLDER}/mirror/bleibt.txt").isFile)
        assertFalse("Papierkorb gespiegelt", File(target, ".SmartExplorer-Papierkorb").exists())
        Api.failure("sync.mirror", args("source" to "trash://", "target" to target.absolutePath), "invalid")
    }

    private fun JsonObject.objectOrNullField(name: String): JsonObject? = this[name] as? JsonObject
}
