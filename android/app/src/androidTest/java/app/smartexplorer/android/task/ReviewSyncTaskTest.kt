package app.smartexplorer.android.task

import android.os.Process
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.system.HostMonitor
import java.io.Closeable
import java.io.File
import java.io.FileOutputStream
import java.nio.file.Files
import java.util.Comparator
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/** AND-SYNC JNI acceptance; the RV1 runner owns prepare → real force-stop → retry. */
@RunWith(AndroidJUnit4::class)
class ReviewSyncTaskTest {
    @Test
    fun savedJobAttemptsKeepChecksAndAccessFailuresDistinct() = coreTest {
        isolated("state") { case ->
            val a = case.dir("a"); val b = case.dir("b")
            Fixture.write(File(a, "state.txt"), "old bytes")
            val id = case.save(a.absolutePath, b.absolutePath)
            assertEquals(JsonNull, state(id).field("lastSuccessMs"))
            case.sync(id)
            assertBytes(File(b, "state.txt"), "old bytes")
            val successful = state(id)
            assertTrue(successful.long("lastAttemptMs") >= successful.long("lastSuccessMs"))
            assertTrue(successful.text("lastRunner").isNotBlank())
            assertEquals("manual", successful.text("lastCause"))
            assertEquals(0, successful.int("consecutiveFailures"))
            assertEquals(JsonNull, successful.field("running"))
            assertNotNull(SyncJobs.byId(id)["lastResult"] as? JsonObject)
            case.run("sync.checkConflicts", args("id" to id))
            assertRegularUnchanged(successful, state(id))

            Fixture.write(File(a, "state.txt"), "new bytes, with a different size")
            waitFor("next native seconds timestamp", 5_000) {
                System.currentTimeMillis() / 1000 * 1000 > successful.long("lastSuccessMs")
            }
            reportStorage(false)
            val denied = case.await(case.start("sync.run", args("id" to id)))
            assertTrue("Unexpected access-denial task: $denied", denied.state in setOf("failed", "canceled"))
            val failed = state(id)
            assertEquals(successful.field("lastSuccessMs"), failed.field("lastSuccessMs"))
            assertTrue(failed.long("lastAttemptMs") > failed.long("lastSuccessMs"))
            assertEquals(successful.int("consecutiveFailures") + 1, failed.int("consecutiveFailures"))
            assertEquals("access", failed.field("lastError").obj().text("kind"))
            assertEquals("needs_action", failed.text("problem"))
            assertBytes(File(b, "state.txt"), "old bytes")
            reportStorage(true)
            case.sync(id)
            assertBytes(File(b, "state.txt"), "new bytes, with a different size")
            assertEquals(0, state(id).int("consecutiveFailures"))
            assertEquals(JsonNull, state(id).field("lastError"))
        }
    }

    @Test
    fun checkedDeleteBlockConsumesOnlyOneMatchingConfirmation() = coreTest {
        isolated("block") { case ->
            val a = case.dir("a"); val b = case.dir("b")
            repeat(9) { Fixture.write(File(a, "$it.txt"), "preserve-$it") }
            val id = case.save(a.absolutePath, b.absolutePath, "maxDelete" to 1,
                "maxDeletePct" to 100, "maxDeleteMin" to 1)
            case.sync(id)
            val successful = state(id)
            assertTrue(File(a, "0.txt").delete()); assertTrue(File(a, "1.txt").delete())
            case.run("sync.checkConflicts", args("id" to id))
            val oldKind = block(id).field("kind")
            assertFalse(block(id).bool("confirmed"))
            assertRegularUnchanged(successful, state(id))
            Api.failure("sync.run", args("id" to id), "blocked")
            repeat(9) { assertBytes(File(b, "$it.txt"), "preserve-$it") }

            assertTrue(File(a, "2.txt").delete())
            case.run("sync.checkConflicts", args("id" to id))
            val currentKind = block(id).field("kind")
            assertFalse("Changed deletion budget retained stale kind", oldKind == currentKind)
            Api.failure("sync.confirmBlock", args("id" to id, "kind" to oldKind))
            assertFalse(block(id).bool("confirmed"))
            Api.call("sync.confirmBlock", args("id" to id, "kind" to currentKind))
            assertTrue(block(id).bool("confirmed"))
            val applied = case.sync(id)
            assertEquals(3, applied.int("deleted"))
            repeat(3) { assertFalse(File(b, "$it.txt").exists()) }
            (3..8).forEach { assertBytes(File(b, "$it.txt"), "preserve-$it") }
            assertFalse((state(id)["blocked"] as? JsonObject)?.bool("confirmed") == true)

            assertTrue(File(a, "3.txt").delete()); assertTrue(File(a, "4.txt").delete())
            case.run("sync.checkConflicts", args("id" to id))
            assertFalse(block(id).bool("confirmed"))
            Api.failure("sync.run", args("id" to id), "blocked")
            (3..8).forEach { assertBytes(File(b, "$it.txt"), "preserve-$it") }
        }
    }

    @Test
    fun manifestVersionRestoreRejectsStaleTokens() = coreTest {
        isolated("versions") { case ->
            val a = case.dir("a"); val b = case.dir("b")
            val old = "version zero"; val newer = "version one, deliberately longer"
            Fixture.write(File(a, "version.txt"), old)
            Fixture.write(File(a, "keep.txt"), "untouched")
            val id = case.save(a.absolutePath, b.absolutePath, "versioning" to true)
            case.sync(id)
            Fixture.write(File(a, "version.txt"), newer)
            case.sync(id)
            val successful = state(id)
            suspend fun listed(): List<JsonObject> = case.run("sync.versions", args("id" to id)).resultObj().objects("items")
            fun selected(items: List<JsonObject>) = items.single { it.text("path") == "version.txt" &&
                it.textOrNull("side") == "b" && it.textOrNull("reason") == "replaced" && it.long("size") == old.toByteArray().size.toLong() }
            val stale = selected(listed()).text("token")
            val chosen = selected(listed())
            assertFalse(stale == chosen.text("token"))
            Api.failure("sync.restoreVersion", args("id" to id, "token" to stale), "invalid")
            assertBytes(File(b, "version.txt"), newer)
            val restored = case.run("sync.restoreVersion", args("id" to id, "token" to chosen.text("token"))).resultObj()
            assertTrue(restored.bool("restored")); assertEquals("version.txt", restored.text("path"))
            assertEquals("b", restored.text("side"))
            assertBytes(File(b, "version.txt"), old); assertBytes(File(a, "version.txt"), newer)
            assertBytes(File(a, "keep.txt"), "untouched"); assertBytes(File(b, "keep.txt"), "untouched")
            assertRegularUnchanged(successful, state(id))
            case.sync(id)
            assertBytes(File(a, "version.txt"), old); assertBytes(File(b, "version.txt"), old)
        }
    }

    @Test
    fun sharedStorageRevokeStopsOnlySharedJobs() = coreTest {
        isolated("storage") { case ->
            val a = case.dir("a"); val b = case.dir("b")
            // Appprivate user data is allowed; the whole cache and filesDir/smart_explorer
            // remain protected own data. InitConfig passes these exact Context directories.
            val privateRoot = File(appContext.filesDir, "review-sync-${case.root.name}")
            val protectedCache = File(Api.obj("sys.info").text("cacheDir")).canonicalPath + File.separator
            assertFalse("Private fixture lies in protected cache", privateRoot.canonicalPath.startsWith(protectedCache))
            assertTrue(privateRoot.mkdirs()); case.directories.add(privateRoot)
            val privateA = File(privateRoot, "a").apply { assertTrue(mkdirs()) }
            val privateB = File(privateRoot, "b").apply { assertTrue(mkdirs()) }
            Fixture.write(File(a, "payload.txt"), "shared payload")
            Fixture.write(File(b, "keep.txt"), "shared counterpart stays")
            Fixture.write(File(privateA, "private.txt"), "independent private bytes")
            val sharedGate = case.gate(privateRoot, "shared")
            val privateGate = case.gate(privateRoot, "private")
            val shared = case.save(a.absolutePath, b.absolutePath, "runBefore" to sharedGate.command)
            val private = case.save(privateA.absolutePath, privateB.absolutePath, "runBefore" to privateGate.command)
            val sharedTask = case.start("sync.run", args("id" to shared))
            val privateTask = case.start("sync.run", args("id" to private))
            waitFor("both actual before-hooks are active", 20_000) {
                sharedGate.ready.isFile && privateGate.ready.isFile && Api.task(sharedTask).isActive &&
                    Api.task(privateTask).isActive && state(shared)["running"] is JsonObject && state(private)["running"] is JsonObject
            }
            val runMark = state(shared).field("running").obj()
            reportStorage(false)
            assertEquals("canceled", case.await(sharedTask, 20_000).state)
            assertTrue("Private run was canceled by Shared revocation", Api.task(privateTask).isActive)
            assertTrue(state(private)["running"] is JsonObject)
            privateGate.release()
            val privateResult = case.await(privateTask, 30_000)
            assertEquals("$privateResult", "done", privateResult.state)
            assertEquals(0, privateResult.resultObj().int("errors"))
            assertBytes(File(privateB, "private.txt"), "independent private bytes")
            val canceled = state(shared)
            assertEquals(JsonNull, canceled.field("lastSuccessMs"))
            assertEquals(0, canceled.int("consecutiveFailures"))
            assertEquals(runMark.field("runner"), canceled.field("lastRunner"))
            assertEquals(runMark.field("cause"), canceled.field("lastCause"))
            assertEquals(JsonNull, canceled.field("running"))
            assertFalse(File(b, "payload.txt").exists())
            assertBytes(File(b, "keep.txt"), "shared counterpart stays")
            reportStorage(true); sharedGate.release()
            case.sync(shared)
            assertBytes(File(b, "payload.txt"), "shared payload")
        }
    }

    @Test
    fun recordedMergeRetrySurvivesProcessRestart() = coreTest {
        when (val phase = TaskArgs.get("reviewMergePhase")) {
            "prepare" -> prepareMerge()
            "retry" -> retryMerge()
            else -> throw AssertionError("Unknown reviewMergePhase=$phase (prepare/retry)")
        }
    }

    private suspend fun prepareMerge() {
        assertFalse("Prior merge session needs retry/cleanup: $session", session.exists())
        val case = newCase("merge")
        var retain = false; var failure: Throwable? = null
        try {
            val server = ReviewMergeFtpFixture(username = case.root.name).also { case.resources.add(it) }
            val a = case.dir("a")
            val base = "Line 1\r\nOriginal\r\nLine 3\r\n"
            val originalA = "Line 1\r\nA\r\nLine 3\r\n"
            val originalB = "Line 1\r\nBBB\r\nLine 3\r\n"
            val expected = "Line 1\r\nA\r\nBBB\r\nLine 3\r\n".toByteArray(Charsets.UTF_8)
            Fixture.write(File(a, MERGE_NAME), base)
            val input = args("protocol" to "ftp", "host" to "127.0.0.1", "port" to server.port,
                "user" to server.user, "password" to ReviewMergeFtpFixture.PASSWORD, "root" to server.root,
                "auth" to "password", "label" to case.root.name)
            val connection = Api.obj("conn.save", args("input" to input))
            case.connections.add(connection.text("id"))
            Api.call("conn.test", args("input" to input))
            val id = case.save(a.absolutePath, connection.text("location"), "versioning" to true)
            case.sync(id)
            assertArrayEquals(base.toByteArray(), server.bytes(MERGE_NAME))
            Fixture.write(File(a, MERGE_NAME), originalA); server.write(MERGE_NAME, originalB)
            case.run("sync.checkConflicts", args("id" to id))
            val conflict = conflict(id)
            val cid = conflict.field("cid") as? JsonPrimitive ?: throw AssertionError("Not an opaque cid: $conflict")
            val rows = Api.obj("sync.mergeRows", args("id" to id, "cid" to cid)).objects("rows")
            assertTrue(rows.isNotEmpty())
            val choices = rows.map { row -> mapOf("takeA" to (row.textOrNull("a") != null),
                "takeB" to (!row.bool("equal") && row.textOrNull("b") != null)) }
            server.refuseNextPublication(MERGE_NAME, expected)
            val failedTask = case.start("sync.mergeApply", args("id" to id, "cid" to cid, "rows" to choices))
            val failed = case.await(failedTask)
            assertEquals("$failed", "failed", failed.state)
            val partial = failed.resultObj()
            TaskReport.note("review-merge-publication", "task=$failed result=$partial")
            assertTrue("Missing partial publication: task=$failed result=$partial", partial.bool("partial"))
            assertTrue("First merge side not confirmed: task=$failed result=$partial", partial.bool("confirmedA"))
            assertFalse(partial.bool("confirmedB")); assertFalse(partial.bool("baselineRecorded"))
            assertTrue(partial.bool("reload")); assertTrue(partial.bool("retry"))
            assertEquals(1, server.rejectedPublishes)
            assertArrayEquals(expected, File(a, MERGE_NAME).readBytes())
            assertArrayEquals(originalB.toByteArray(), server.bytes(MERGE_NAME))
            server.assertHealthy()
            val saved = args("version" to 1, "pid" to Process.myPid(), "failedTask" to failedTask, "failedStartedMs" to failed.startedMs,
                "root" to case.root.absolutePath, "source" to a.absolutePath, "target" to connection.text("location"),
                "job" to id, "connection" to connection.text("id"), "port" to server.port,
                "originalA" to ReviewMergeFtpFixture.encode(originalA.toByteArray()),
                "originalB" to ReviewMergeFtpFixture.encode(originalB.toByteArray()),
                "merged" to ReviewMergeFtpFixture.encode(expected), "ftp" to server.snapshot())
            writeSession(saved)
            case.sessionOwned = true
            TaskReport.note("review-merge-prepare", "pid=${Process.myPid()} job=$id port=${server.port} partial=$partial; session=$session")
            retain = true
        } catch (e: Throwable) { failure = e; throw e }
        finally { finish(case, retain, failure) }
    }

    private suspend fun retryMerge() {
        assertTrue("prepare session missing: $session", session.isFile)
        assertTrue("Oversized fixture session", session.length() <= 1024 * 1024)
        val saved = Core.json.parseToJsonElement(session.readText()).obj()
        assertEquals(1, saved.int("version"))
        val root = File(saved.text("root"))
        val allowed = File(Volumes.primary().path, "${Fixture.FOLDER}/review-sync").canonicalPath + File.separator
        assertTrue("Foreign fixture root", root.canonicalPath.startsWith(allowed))
        val source = File(saved.text("source"))
        assertEquals(File(root, "a").canonicalPath, source.canonicalPath)
        val case = Case(root).apply { jobs.add(saved.text("job")); connections.add(saved.text("connection")); sessionOwned = true }
        var failure: Throwable? = null
        try {
            assertFalse("Runner did not restart the actual app process", saved.int("pid") == Process.myPid())
            initializeHost()
            val server = ReviewMergeFtpFixture(saved.int("port"), saved.field("ftp").obj()).also { case.resources.add(it) }
            assertEquals(saved.int("port"), server.port)
            assertFalse("Old task snapshot survived the process restart", Api.objects("task.list").any {
                it.text("id") == saved.text("failedTask") && it.long("startedMs") == saved.long("failedStartedMs")
            })
            val id = saved.text("job")
            val job = SyncJobs.byId(id)
            assertEquals(saved.text("source"), job.text("source"))
            assertEquals(saved.text("target"), job.text("target"))
            assertFalse(Api.obj("sync.conflicts", args("id" to id)).bool("available"))
            val expected = ReviewMergeFtpFixture.decode(saved.text("merged"))
            assertArrayEquals(expected, File(source, MERGE_NAME).readBytes())
            assertArrayEquals(ReviewMergeFtpFixture.decode(saved.text("originalB")), server.bytes(MERGE_NAME))
            case.run("sync.checkConflicts", args("id" to id))
            val recovered = conflict(id)
            assertTrue(recovered.bool("pendingMerge"))
            assertEquals(ReviewMergeFtpFixture.decode(saved.text("originalA")).size.toLong(), recovered.field("a").obj().long("size"))
            assertEquals(ReviewMergeFtpFixture.decode(saved.text("originalB")).size.toLong(), recovered.field("b").obj().long("size"))
            val cid = recovered.field("cid")
            val view = Api.obj("sync.mergeRows", args("id" to id, "cid" to cid))
            assertTrue("Published bytes became a fresh draft", view.objects("rows").isEmpty())
            val pending = view.field("pending").obj()
            assertEquals("write", pending.text("kind"))
            assertEquals(expected.toString(Charsets.UTF_8), pending.text("preview"))
            assertFalse(pending.bool("previewTruncated")); assertTrue(pending.bool("confirmedA")); assertFalse(pending.bool("confirmedB"))
            assertArrayEquals(expected, File(source, MERGE_NAME).readBytes())
            assertArrayEquals(ReviewMergeFtpFixture.decode(saved.text("originalB")), server.bytes(MERGE_NAME))
            val result = case.run("sync.mergeRetry", args("id" to id, "cid" to cid)).resultObj()
            assertTrue(result.bool("confirmedA")); assertTrue(result.bool("confirmedB"))
            assertTrue(result.bool("baselineRecorded")); assertFalse(result.bool("partial")); assertFalse(result.bool("retry"))
            assertEquals(0, result.int("remaining"))
            assertArrayEquals(expected, File(source, MERGE_NAME).readBytes()); assertArrayEquals(expected, server.bytes(MERGE_NAME))
            case.sync(id)
            case.run("sync.checkConflicts", args("id" to id))
            assertTrue(Api.obj("sync.conflicts", args("id" to id)).objects("items").isEmpty())
            assertEquals(0, server.rejectedPublishes); server.assertHealthy()
            TaskReport.note("review-merge-retry", "preparePid=${saved.int("pid")} retryPid=${Process.myPid()} port=${server.port} result=$result")
        } catch (e: Throwable) { failure = e; throw e }
        finally { finish(case, false, failure) }
    }

    private suspend fun isolated(name: String, work: suspend (Case) -> Unit) {
        val case = newCase(name); var failure: Throwable? = null
        try { work(case) } catch (e: Throwable) { failure = e; throw e }
        finally { finish(case, false, failure) }
    }
    private suspend fun newCase(name: String): Case {
        initializeHost()
        return Case(Fixture.dir(Volumes.primary(), "review-sync", Fixture.unique(name)))
    }
    private suspend fun initializeHost() {
        assertTrue("Shared Storage permission required", HostMonitor.measure(appContext).storageAccess)
        HostMonitor.start(appContext)
        waitFor("initial measured host report", 20_000) { HostMonitor.hostState.value?.storageAccess == true }
        reportStorage(true)
    }
    private suspend fun reportStorage(granted: Boolean) {
        val host = HostMonitor.measure(appContext).copy(storageAccess = granted, deferScheduling = true)
        SyncApi.hostState(host)
        TaskReport.called("sys.hostState", "ok")
        assertEquals(granted, Api.obj("bg.status").bool("storageAccess"))
    }
    private suspend fun state(id: String) = SyncJobs.byId(id).field("state").obj()
    private suspend fun block(id: String) = state(id).field("blocked").obj()
    private suspend fun conflict(id: String): JsonObject {
        val answer = Api.obj("sync.conflicts", args("id" to id))
        assertTrue(answer.bool("available"))
        return answer.objects("items").single { it.text("path") == MERGE_NAME }
    }
    private fun assertRegularUnchanged(before: JsonObject, after: JsonObject) {
        listOf("lastAttemptMs", "lastSuccessMs", "lastRunner", "lastCause", "consecutiveFailures", "lastError").forEach {
            assertEquals("Pure operation changed $it", before.field(it), after.field(it))
        }
    }
    private fun assertBytes(file: File, text: String) = assertArrayEquals(file.absolutePath, text.toByteArray(Charsets.UTF_8), file.readBytes())
    private val session get() = TaskReport.file("review-sync-merge-session.json")
    private fun writeSession(value: JsonObject) {
        val temporary = File(session.parentFile, session.name + ".tmp")
        FileOutputStream(temporary).use { out -> out.write(value.toString().toByteArray(Charsets.UTF_8)); out.fd.sync() }
        assertTrue("Session checkpoint failed", temporary.renameTo(session))
    }

    private class Gate(val ready: File, val released: File) {
        private fun quote(file: File) = "'" + file.absolutePath.replace("'", "'\\''") + "'"
        val command get() = "set -e; printf ready > ${quote(ready)}; n=0; while [ ! -f ${quote(released)} ]; do " +
            "n=${'$'}((n+1)); [ ${'$'}n -lt 600 ] || exit 91; sleep 0.1; done"
        fun release() { Fixture.write(released, "release") }
    }
    private class Case(val root: File) {
        val jobs = mutableListOf<String>(); val connections = mutableListOf<String>()
        val tasks = mutableListOf<String>(); val directories = mutableListOf(root)
        val gates = mutableListOf<Gate>(); val resources = mutableListOf<Closeable>()
        var sessionOwned = false
        fun dir(name: String) = File(root, name).apply { assertTrue(mkdirs()) }
        fun gate(parent: File, name: String) = Gate(File(parent, "$name.ready"), File(parent, "$name.release")).also { gates.add(it) }
        suspend fun save(source: String, target: String, vararg extra: Pair<String, Any?>): String = SyncJobs.save(
            "name" to root.name, "source" to source, "target" to target, "direction" to "both",
            "conflict" to "strict", "deletePolicy" to "propagate", "trigger" to "manual",
            "enabled" to true, "useRecycleBin" to false, *extra,
        ).text("id").also { jobs.add(it) }
        suspend fun start(method: String, input: JsonObject) = Api.start(method, input).also { tasks.add(it) }
        suspend fun await(id: String, timeoutMs: Long = Api.TASK_TIMEOUT_MS) = Api.await(id, timeoutMs)
        suspend fun run(method: String, input: JsonObject): TaskInfo = await(start(method, input)).also {
            assertEquals("$method: $it", "done", it.state)
        }
        suspend fun sync(id: String): JsonObject = run("sync.run", args("id" to id)).resultObj().also {
            assertEquals("Sync errors: $it", 0, it.int("errors")); assertEquals("Sync conflicts: $it", 0, it.int("conflicts"))
        }
    }

    private suspend fun finish(case: Case, retain: Boolean, primary: Throwable?) {
        val errors = mutableListOf<Throwable>()
        withContext(NonCancellable) {
            try {
                withTimeout(60_000) {
                    case.gates.forEach { it.release() }
                    suspend fun current(id: String): TaskInfo? = try { Api.task(id) } catch (e: CoreException) {
                        if (e.kind == "not_found") null else throw e
                    }
                    case.tasks.forEach { id -> if (current(id)?.isActive == true) Api.call("task.cancel", args("id" to id)) }
                    case.tasks.forEach { if (current(it) != null) Api.await(it, 30_000) }
                    waitFor("own job workers released", 30_000) { case.jobs.all { state(it)["running"] == JsonNull } }
                    if (!retain) {
                        case.jobs.forEach { Api.call("sync.delete", args("id" to it)) }
                        case.connections.forEach { Api.call("conn.delete", args("id" to it)) }
                        case.directories.asReversed().forEach { directory ->
                            if (directory.exists()) Files.walk(directory.toPath()).use { paths ->
                                paths.sorted(Comparator.reverseOrder()).forEach { Files.deleteIfExists(it) }
                            }
                        }
                        if (case.sessionOwned) assertTrue("Session cleanup failed", session.delete())
                        if (case.root.name.startsWith("merge-")) {
                            val temporary = File(session.parentFile, session.name + ".tmp")
                            if (temporary.exists()) assertTrue(temporary.delete())
                        }
                    }
                }
            } catch (e: Throwable) { errors.add(e) }
            try { withTimeout(10_000) { SyncApi.hostState(HostMonitor.measure(appContext)) } }
            catch (e: Throwable) { errors.add(e) }
            case.resources.asReversed().forEach { resource -> try { resource.close() } catch (e: Throwable) { errors.add(e) } }
        }
        if (errors.isNotEmpty()) {
            errors.forEach { runCatching { TaskReport.note("review-sync-cleanup", it.toString()) } }
            if (primary != null) errors.forEach { primary.addSuppressed(it) }
            else { errors.drop(1).forEach { errors.first().addSuppressed(it) }; throw errors.first() }
        }
    }

    companion object { private const val MERGE_NAME = "notiz.txt" }
}
