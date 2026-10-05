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
    // Published 0.5.169 rejects linked ancestors, including Android's filesDir alias.
    // Resolve our fixture before the old job is saved; keep that stored locator across the update.
    private val filesDir get() = appContext.filesDir.canonicalFile
    private val root get() = File(filesDir, "sync-reliability-fixture")
    private val session get() = File(root, "session.json")
    private val a get() = File(root, "a")
    private val b get() = File(root, "b")
    private val outputFields = setOf("lastRun", "lastResult", "schedule", "runningTask", "state", "brokenConfig")
    private fun configFile(id: String) = File(filesDir, "smart_explorer/sync/jobs/$id.conf")
    private fun config(body: String): Map<String, List<String>> = body.lineSequence()
        .filter { it.isNotBlank() && !it.trimStart().startsWith("#") }
        .map { line ->
            assertTrue("Invalid actual job configuration line: $line", line.contains('='))
            line.substringBefore('=') to line.substringAfter('=')
        }.groupBy({ it.first }, { it.second })

    /** Every historical key, including settings the mobile JSON facade does not expose. */
    private fun preservedConfig(record: JsonObject, id: String): Boolean {
        val body = record.text("configBody")
        val old = config(body); val file = configFile(id); val currentBody = file.readText(); val now = config(currentBody)
        val migrating = "config_version" !in old
        val expected = old.toMutableMap()
        if (migrating) {
            assertEquals("Historical delete threshold is malformed", 1, old.getValue("max_delete_pct").size)
            val threshold = old.getValue("max_delete_pct").single().toLong()
            if (threshold == 0L) expected["max_delete_pct"] = listOf("50")
            expected.putAll(mapOf(
                "config_version" to listOf("1"), "rt_max_latency_secs" to listOf("0"),
                "rt_poll_secs" to listOf("300"), "verify_interval_secs" to listOf("3600"),
                "verify_target_secs" to listOf("86400"), "max_delete_min" to listOf(if (threshold == 0L) "25" else "0"),
                "versions_location" to listOf("auto"), "cross_mounts" to listOf("1"), "run_cleanup" to listOf("")))
        }
        val differences = (expected.keys + now.keys).filter { expected[it] != now[it] }
            .associateWith { args("expected" to expected[it], "actual" to now[it]) }
        TaskReport.note("sync-reliability-config", args("id" to id, "migrating" to migrating,
            "oldSha256" to record.field("persisted").obj()[file.relativeTo(filesDir).path],
            "currentSha256" to Fixture.sha256(file), "differences" to differences).toString())
        assertEquals("Actual stored setting differences: $differences", expected, now)
        if (!migrating) assertEquals("Unexpected config byte change after completed migration", body, currentBody)
        if (migrating) {
            val pending = File(filesDir, "smart_explorer/sync/legacy-baseline-jobs/$id.pending")
            assertTrue("Missing exact-job legacy baseline eligibility", pending.isFile)
            val endpoints = Core.json.parseToJsonElement(pending.readText()) as JsonArray
            assertEquals(jsonOf(listOf(record.field("job").obj().text("source"), record.field("job").obj().text("target"))), endpoints)
        }
        return migrating
    }


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
        val diagnostics = call("task.get", args("id" to task)).obj()
        assertEquals("$method: $diagnostics", "done", done.state)
        return done.resultObj().also {
            if (method == "sync.run") assertEquals("$method: $diagnostics", 0, it.int("errors"))
        }
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
    private fun isBaseline(file: File) = file.extension == "sebl" &&
        (file.name.startsWith("baseline_") || file.path.contains("/sync/pairs/"))
    private fun validateBaseline(path: String) {
        val file = File(filesDir, path)
        assertTrue("Not a pair baseline: $path", isBaseline(file))
        file.inputStream().use { input ->
            val magic = ByteArray(5); assertEquals(5, input.read(magic))
            assertArrayEquals(byteArrayOf(83, 69, 66, 76, 2), magic)
        }
    }
    private fun persisted(): Map<String, String> = File(filesDir, "smart_explorer").walkTopDown()
        .filter { it.isFile && (it.extension in setOf("conf", "tsv") || isBaseline(it) ||
            (it.extension == "journal" && it.path.contains("/sync/pairs/"))) }
        .associate { it.relativeTo(filesDir).path to Fixture.sha256(it) }

    private suspend fun oldPrepare() {
        assertEquals("0.5.169", appContext.packageManager.getPackageInfo(appContext.packageName, 0).versionName)
        assertFalse(root.canonicalPath.startsWith(File(filesDir, "smart_explorer").canonicalPath + File.separator))
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
        val backupRoots = listOf(root, File(filesDir, "smart_explorer/sync"))
        val backups = backupRoots.flatMap { dir -> dir.walkTopDown().filter { it.isFile &&
            (it.path.contains("/versions_") || it.path.contains("/.se-versions/")) &&
            it.readBytes().contentEquals("old seed".toByteArray()) }.map { it.relativeTo(filesDir).path }.toList() }
        assertTrue("Published job did not preserve displaced bytes in its legacy appdata store", backups.any {
            it.startsWith("smart_explorer/sync/versions_")
        })
        checkpoint(args("job" to job(id), "pid" to Process.myPid(), "persisted" to state, "backups" to backups, "configBody" to configFile(id).readText()), "old-prepare")
    }
    private suspend fun preserved(record: JsonObject): String {
        val old = record.field("job").obj(); val id = old.text("id"); val now = job(id)
        val migrating = preservedConfig(record, id)
        old.filterKeys { it !in outputFields }.forEach { (field, value) ->
            val expected = if (migrating && field == "maxDeletePct" && old.int(field) == 0) jsonOf(50) else value
            assertEquals("Stored API option $field (old=$old current=$now)", expected, now[field])
        }
        record.field("persisted").obj().forEach { (path, hash) ->
            val file = File(filesDir, path)
            // Only this exact job configuration has a documented one-time RV1 rewrite.
            if (!(migrating && file.canonicalPath == configFile(id).canonicalPath)) {
                assertEquals("Persisted state $path", hash, jsonOf(Fixture.sha256(file)))
            }
        }
        record.texts("backups").forEach { assertArrayEquals("old seed".toByteArray(), File(filesDir, it).readBytes()) }
        return id
    }
    private suspend fun updatePrepare() {
        val record = Core.json.parseToJsonElement(session.readText()).obj(); val id = preserved(record)
        val migratedBody = configFile(id).readText()
        assertNotEquals(record.int("pid"), Process.myPid()); bytes(a, "published changed bytes"); bytes(b, "published changed bytes")
        val noop = sync(id); assertEquals(0, noop.int("aToB")); assertEquals(0, noop.int("bToA"))
        assertEquals("Migration No-op deleted files", 0, noop.int("deleted"))
        val legacy = record.field("persisted").obj().filterKeys { File(it).name.startsWith("baseline_") && File(it).extension == "sebl" }
        assertEquals("Ambiguous real published pair state", 1, legacy.size)
        legacy.forEach { (path, hash) ->
            assertEquals("No-op changed published pair baseline $path", hash, jsonOf(Fixture.sha256(File(filesDir, path))))
        }
        val owners = File(filesDir, "smart_explorer/sync/pairs").walkTopDown().filter {
            it.isFile && it.extension == "sebl" && it.name.startsWith("job-$id.")
        }.toList()
        assertEquals("Expected one real imported owner/replica baseline for saved job $id: $owners", 1, owners.size)
        val owner = owners.single(); val ownerPath = owner.relativeTo(filesDir).path
        validateBaseline(ownerPath)
        assertEquals("Owner import changed the confirmed published baseline records", legacy.values.single(), jsonOf(Fixture.sha256(owner)))
        val ownerState = args("path" to ownerPath, "pair" to owner.parentFile!!.name, "owner" to "job-$id",
            "replicas" to owner.name.removePrefix("job-$id.").removeSuffix(".sebl"), "sha256" to Fixture.sha256(owner))
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
                assertEquals("Failed apply changed baseline $path", hash, Fixture.sha256(File(filesDir, path)))
            }
        } finally { assertTrue("Cannot restore fixture target writes", b.setWritable(true, true)) }
        // Retry the same saved job through JNI after restoring actual filesystem access.
        sync(id); bytes(b, "temporary failure change")
        assertTrue("Failure removed old baseline", beforeFailure.keys.all { File(filesDir, it).isFile })
        write(File(a, "note.txt"), "candidate side A"); write(File(b, "note.txt"), "candidate side B longer")
        run("sync.checkConflicts", id)
        assertEquals(1, call("sync.conflicts", args("id" to id)).obj().objects("items").size)
        checkpoint(record.withFields("pid" to Process.myPid(), "persisted" to persisted(), "job" to job(id),
            "configBody" to configFile(id).readText(), "ownerState" to ownerState,
            "migration" to args("fromBody" to record.text("configBody"), "toBody" to migratedBody,
                "afterRunsBody" to configFile(id).readText())), "update-prepare")
    }
    private suspend fun retry() {
        val record = Core.json.parseToJsonElement(session.readText()).obj(); val id = preserved(record)
        val owner = record.field("ownerState").obj()
        assertEquals("Restart lost saved job owner", "job-$id", owner.text("owner"))
        assertTrue("Restart lost actual owner baseline", File(filesDir, owner.text("path")).isFile)
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
