package app.smartexplorer.android.task

import android.os.Bundle
import android.os.Process
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.smartexplorer.android.core.Core
import java.io.File
import java.io.FileOutputStream
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** The host installs the published APK first; no run-as or fabricated job persistence. */
@RunWith(AndroidJUnit4::class)
class SyncReliabilityTaskTest {
    private val root get() = File(appContext.filesDir, "sync-reliability-fixture")
    private val session get() = File(root, "session.json")
    private val a get() = File(root, "a")
    private val b get() = File(root, "b")
    private val fields = listOf("id", "name", "source", "target", "direction", "conflict", "deletePolicy",
        "compare", "versioning", "retainDays", "trigger", "includeHidden", "ignore", "enabled", "useRecycleBin")

    @Test fun publishedJobUpdateAndRestart() = coreTest {
        when (TaskArgs.get("syncReliabilityPhase")) {
            "old-prepare" -> oldPrepare()
            "update-prepare" -> updatePrepare()
            "retry" -> retry()
            else -> throw AssertionError("Unknown syncReliabilityPhase")
        }
    }

    private suspend fun call(method: String, input: JsonObject = NO_ARGS) = Core.call(method, input)
    private suspend fun run(method: String, id: String): JsonObject {
        val task = call(method, args("id" to id)).obj().text("taskId")
        val done = Api.await(task)
        assertEquals("$method: $done", "done", done.state)
        return done.resultObj()
    }
    private suspend fun job(id: String) = (call("sync.jobs") as JsonArray).map { it.obj() }.single { it.text("id") == id }
    private suspend fun sync(id: String) = run("sync.run", id).also {
        assertEquals("$it", 0, it.int("errors")); assertEquals("$it", 0, it.int("conflicts"))
    }
    private fun write(file: File, text: String) {
        FileOutputStream(file).use { it.write(text.toByteArray()); it.fd.sync() }
    }
    private fun bytes(dir: File, text: String) = assertArrayEquals(text.toByteArray(), File(dir, "note.txt").readBytes())
    private fun checkpoint(record: JsonObject, phase: String) {
        write(session, record.toString())
        // Host reads this through instrumentation output even while old release is non-debuggable.
        val message = "SYNC_RELIABILITY_MARKER " + record.withFields("phase" to phase) + "\n"
        InstrumentationRegistry.getInstrumentation().sendStatus(0, Bundle().apply { putString("stream", message) })
    }
    private fun isBaseline(file: File) = file.name.startsWith("baseline_") && file.extension == "sebl"
    private fun validateBaseline(path: String) {
        val file = File(appContext.filesDir, path)
        assertTrue("Not a pair baseline: $path", isBaseline(file))
        file.inputStream().use { input ->
            val magic = ByteArray(5); assertEquals(5, input.read(magic))
            assertArrayEquals(byteArrayOf(83, 69, 66, 76, 2), magic)
        }
    }
    private fun persisted(): Map<String, String> = File(appContext.filesDir, "smart_explorer").walkTopDown()
        .filter { it.isFile && (it.extension in setOf("conf", "tsv") || isBaseline(it)) }
        .associate { it.relativeTo(appContext.filesDir).path to Fixture.sha256(it) }

    private suspend fun oldPrepare() {
        assertEquals("0.5.169", appContext.packageManager.getPackageInfo(appContext.packageName, 0).versionName)
        assertFalse(root.canonicalPath.startsWith(File(appContext.filesDir, "smart_explorer").canonicalPath + File.separator))
        assertFalse(root.canonicalPath.startsWith(appContext.cacheDir.canonicalPath + File.separator))
        assertFalse("Fixture already exists", root.exists()); assertTrue(a.mkdirs()); assertTrue(b.mkdirs())
        write(File(a, "note.txt"), "old seed")
        val options = call("sync.options").obj()
        val versioning = options.objects("versionings").first { it.text("value") !in setOf("none", "off", "false") }.text("value")
        val saved = call("sync.save", args("job" to options.field("defaults").obj().withFields(
            "id" to "", "name" to "Published Android preserved job", "source" to a.absolutePath,
            "target" to b.absolutePath, "direction" to "both", "conflict" to "strict", "compare" to "checksum",
            "versioning" to versioning, "trigger" to "manual", "includeHidden" to true,
            "ignore" to listOf("*.ignored"), "useRecycleBin" to false))).obj()
        val id = saved.text("id"); sync(id); bytes(b, "old seed")
        write(File(a, "note.txt"), "published changed bytes")
        sync(id); bytes(b, "published changed bytes")
        val state = persisted(); assertTrue("No actual persisted baseline/config", state.size >= 2)
        val baselines = state.keys.filter { isBaseline(File(it)) }
        assertTrue("No actual published pair baseline", baselines.isNotEmpty())
        baselines.forEach { validateBaseline(it) }
        val backupRoots = listOf(root, File(appContext.filesDir, "smart_explorer/sync"))
        val backups = backupRoots.flatMap { dir -> dir.walkTopDown().filter { it.isFile &&
            (it.path.contains("/versions_") || it.path.contains("/.se-versions/")) &&
            it.readBytes().contentEquals("old seed".toByteArray()) }.map { it.relativeTo(appContext.filesDir).path }.toList() }
        assertTrue("Published job did not preserve displaced bytes in its legacy appdata store", backups.any {
            it.startsWith("smart_explorer/sync/versions_")
        })
        checkpoint(args("job" to job(id), "pid" to Process.myPid(), "persisted" to state, "backups" to backups), "old-prepare")
    }
    private suspend fun preserved(record: JsonObject): String {
        val old = record.field("job").obj(); val id = old.text("id"); val now = job(id)
        fields.forEach { assertEquals("Stored option $it", old[it], now[it]) }
        record.field("persisted").obj().forEach { (path, hash) ->
            assertEquals("Persisted state $path", hash, jsonOf(Fixture.sha256(File(appContext.filesDir, path))))
        }
        record.texts("backups").forEach { assertArrayEquals("old seed".toByteArray(), File(appContext.filesDir, it).readBytes()) }
        return id
    }
    private suspend fun updatePrepare() {
        val record = Core.json.parseToJsonElement(session.readText()).obj(); val id = preserved(record)
        assertNotEquals(record.int("pid"), Process.myPid()); bytes(a, "published changed bytes"); bytes(b, "published changed bytes")
        val noop = sync(id); assertEquals(0, noop.int("aToB")); assertEquals(0, noop.int("bToA"))
        val beforeFailure = persisted()
        write(File(a, "note.txt"), "temporary failure change")
        assertTrue("Cannot revoke fixture target writes", b.setWritable(false, false))
        try {
            val task = call("sync.run", args("id" to id)).obj().text("taskId")
            val failed = Api.await(task)
            assertTrue("Unexpected success through unwritable target: $failed",
                failed.state == "failed" || (failed.result as? JsonObject)?.int("errors")?.let { it > 0 } == true)
            bytes(b, "published changed bytes")
            beforeFailure.filterKeys { isBaseline(File(it)) }.forEach { (path, hash) ->
                assertEquals("Failed apply changed baseline $path", hash, Fixture.sha256(File(appContext.filesDir, path)))
            }
        } finally { assertTrue("Cannot restore fixture target writes", b.setWritable(true, true)) }
        // Retry the same saved job through JNI after restoring actual filesystem access.
        sync(id); bytes(b, "temporary failure change")
        assertTrue("Failure removed old baseline", beforeFailure.keys.all { File(appContext.filesDir, it).isFile })
        write(File(a, "note.txt"), "candidate side A"); write(File(b, "note.txt"), "candidate side B longer")
        run("sync.checkConflicts", id)
        assertEquals(1, call("sync.conflicts", args("id" to id)).obj().objects("items").size)
        checkpoint(record.withFields("pid" to Process.myPid(), "persisted" to persisted()), "update-prepare")
    }
    private suspend fun retry() {
        val record = Core.json.parseToJsonElement(session.readText()).obj(); val id = preserved(record)
        assertNotEquals(record.int("pid"), Process.myPid()); bytes(a, "candidate side A"); bytes(b, "candidate side B longer")
        run("sync.checkConflicts", id)
        val conflict = call("sync.conflicts", args("id" to id)).obj().objects("items").single()
        val task = call("sync.resolve", args("id" to id, "cid" to conflict.field("cid"), "choice" to "a")).obj().text("taskId")
        val resolved = Api.await(task); assertEquals("$resolved", "done", resolved.state)
        call("sync.finishConflicts", args("id" to id)); sync(id)
        bytes(a, "candidate side A"); bytes(b, "candidate side A")
        write(File(b, "note.txt"), "restarted reverse change")
        sync(id); bytes(a, "restarted reverse change"); bytes(b, "restarted reverse change")
        val noop = sync(id); assertEquals(0, noop.int("aToB")); assertEquals(0, noop.int("bToA")); assertEquals(0, noop.int("deleted"))
        val versions = run("sync.versions", id).objects("items")
        assertTrue("Legacy appdata version disappeared", versions.any { it.text("store") == "app_data" &&
            it.text("path") == "note.txt" && it.long("size") == "old seed".toByteArray().size.toLong() })
        assertTrue("Auto did not preserve current local displaced bytes at sync root", versions.any {
            it.text("store") == "sync_root" && it.text("path") == "note.txt"
        })
        checkpoint(args("job" to job(id), "pid" to Process.myPid(), "sourceSha256" to Fixture.sha256(File(a, "note.txt")),
            "targetSha256" to Fixture.sha256(File(b, "note.txt")), "versions" to versions, "noop" to noop), "retry")
        call("sync.delete", args("id" to id)); assertTrue(root.deleteRecursively())
    }
}
