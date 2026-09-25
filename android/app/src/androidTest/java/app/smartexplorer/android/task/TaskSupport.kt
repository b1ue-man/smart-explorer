package app.smartexplorer.android.task

import android.content.Context
import androidx.test.platform.app.InstrumentationRegistry
import app.smartexplorer.android.api.Listing
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.CoreState
import app.smartexplorer.android.core.Entry
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.core.VolumeInfo
import app.smartexplorer.android.system.Storage
import java.io.File
import java.security.MessageDigest
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue

// Shared helpers of the Android task suite (android/test-android-task.sh, G4–G6). The tests run
// inside the app process (instrumentation), call the real core through `Core.call` and record
// every api.md method they reach in `<filesDir>/task-report/calls.tsv` for the coverage list.

/** The app under test (the instrumentation shares its process). */
val appContext: Context
    get() = InstrumentationRegistry.getInstrumentation().targetContext

/** Values the host script passes with `am instrument -e <name> <value>`. */
object TaskArgs {
    fun get(name: String): String =
        InstrumentationRegistry.getArguments().getString(name)?.takeIf { it.isNotBlank() }
            ?: throw AssertionError("Instrumentation-Argument fehlt: -e $name <Wert> (android/test-android-task.sh)")

    fun int(name: String): Int = get(name).toIntOrNull() ?: throw AssertionError("Argument $name ist keine Zahl: ${get(name)}")
}

/** Kotlin values → JSON (null, String, Number, Boolean, Map, Iterable, JsonElement). */
fun jsonOf(value: Any?): JsonElement = when (value) {
    null -> JsonNull
    is JsonElement -> value
    is String -> JsonPrimitive(value)
    is Boolean -> JsonPrimitive(value)
    is Number -> JsonPrimitive(value)
    is Map<*, *> -> JsonObject(value.entries.associate { entry -> entry.key.toString() to jsonOf(entry.value) })
    is Iterable<*> -> JsonArray(value.map { jsonOf(it) })
    else -> throw IllegalArgumentException("Kein JSON-Wert: ${value.javaClass.name}")
}

fun args(vararg pairs: Pair<String, Any?>): JsonObject = JsonObject(pairs.associate { (key, value) -> key to jsonOf(value) })

val NO_ARGS: JsonObject = JsonObject(emptyMap())

/** A copy of this object with [changes] applied. */
fun JsonObject.withFields(vararg changes: Pair<String, Any?>): JsonObject = JsonObject(this + args(*changes))

fun JsonElement.obj(): JsonObject = this as? JsonObject ?: throw AssertionError("JSON-Objekt erwartet: $this")

fun JsonObject.field(name: String): JsonElement = this[name] ?: throw AssertionError("Feld \"$name\" fehlt in $this")

fun JsonObject.text(name: String): String =
    (field(name) as? JsonPrimitive)?.contentOrNull ?: throw AssertionError("Feld \"$name\" ist kein Text in $this")

fun JsonObject.textOrNull(name: String): String? = (this[name] as? JsonPrimitive)?.contentOrNull

fun JsonObject.bool(name: String): Boolean =
    (field(name) as? JsonPrimitive)?.booleanOrNull ?: throw AssertionError("Feld \"$name\" ist kein Boolean in $this")

fun JsonObject.long(name: String): Long =
    (field(name) as? JsonPrimitive)?.longOrNull ?: throw AssertionError("Feld \"$name\" ist keine Zahl in $this")

fun JsonObject.int(name: String): Int =
    (field(name) as? JsonPrimitive)?.intOrNull ?: throw AssertionError("Feld \"$name\" ist keine Zahl in $this")

fun JsonObject.array(name: String): JsonArray = field(name) as? JsonArray ?: throw AssertionError("Feld \"$name\" ist kein Array in $this")

fun JsonObject.objects(name: String): List<JsonObject> = array(name).map { it.obj() }

fun JsonObject.texts(name: String): List<String> =
    array(name).map { (it as? JsonPrimitive)?.contentOrNull ?: throw AssertionError("Text erwartet in $name: $it") }

/** Coverage and evidence written for the host script (pulled from `<filesDir>/task-report`). */
object TaskReport {
    private val seen = HashSet<String>()

    val dir: File
        get() = File(appContext.filesDir, "task-report").apply { mkdirs() }

    /** One line per distinct (method, outcome) in this process. */
    @Synchronized
    fun called(method: String, outcome: String) {
        if (seen.add("$method\t$outcome")) File(dir, "calls.tsv").appendText("$method\t$outcome\n")
    }

    @Synchronized
    fun note(topic: String, text: String) {
        File(dir, "notes.txt").appendText("[$topic] $text\n")
    }

    fun file(name: String): File = File(dir, name).also { it.parentFile?.mkdirs() }
}

/** Core calls through the real JNI bridge, recorded for the api.md coverage list. */
object Api {
    private const val POLL_MS = 250L
    const val TASK_TIMEOUT_MS = 180_000L

    suspend fun awaitReady() {
        val state = withTimeout(120_000) { Core.ready.first { it !is CoreState.Starting } }
        if (state is CoreState.Failed) throw AssertionError("Kern nicht gestartet: ${state.message}")
    }

    suspend fun call(method: String, args: JsonObject = NO_ARGS): JsonElement {
        try {
            val result = Core.call(method, args)
            TaskReport.called(method, "ok")
            return result
        } catch (e: CoreException) {
            TaskReport.called(method, "error:${e.kind}")
            throw e
        }
    }

    suspend fun obj(method: String, args: JsonObject = NO_ARGS): JsonObject = call(method, args).obj()

    suspend fun objects(method: String, args: JsonObject = NO_ARGS): List<JsonObject> {
        val result = call(method, args)
        val array = result as? JsonArray ?: throw AssertionError("$method: Array erwartet, erhalten $result")
        return array.map { it.obj() }
    }

    suspend inline fun <reified T> get(method: String, args: JsonObject = NO_ARGS): T =
        Core.json.decodeFromJsonElement<T>(call(method, args))

    /** Expects [method] to fail; [kinds] (if given) lists the acceptable error kinds. */
    suspend fun failure(method: String, args: JsonObject, vararg kinds: String): CoreException {
        var caught: CoreException? = null
        try {
            call(method, args)
        } catch (e: CoreException) {
            caught = e
        }
        val error = caught ?: throw AssertionError("$method $args hätte scheitern müssen")
        if (kinds.isNotEmpty()) {
            assertTrue("$method: Fehlerart ${error.kind} (${error.message}) nicht in ${kinds.toList()}", error.kind in kinds)
        }
        return error
    }

    /** Calls [method] where both outcomes are legitimate (recorded either way for the coverage list). */
    suspend fun attempt(method: String, args: JsonObject = NO_ARGS): Result<JsonElement> =
        try {
            Result.success(call(method, args))
        } catch (e: CoreException) {
            Result.failure(e)
        }

    suspend fun start(method: String, args: JsonObject = NO_ARGS): String = obj(method, args).text("taskId")

    suspend fun task(id: String): TaskInfo = get("task.get", args("id" to id))

    suspend fun await(id: String, timeoutMs: Long = TASK_TIMEOUT_MS): TaskInfo {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (true) {
            val info = task(id)
            if (!info.isActive) return info
            if (System.currentTimeMillis() > deadline) {
                throw AssertionError("Task $id (${info.kind}) nach ${timeoutMs / 1000} s nicht fertig: ${info.state} ${info.message.orEmpty()}")
            }
            delay(POLL_MS)
        }
    }

    /** Starts a task, waits for it and requires `done`. */
    suspend fun runTask(method: String, args: JsonObject = NO_ARGS, timeoutMs: Long = TASK_TIMEOUT_MS): TaskInfo {
        val info = await(start(method, args), timeoutMs)
        assertEquals("$method ${info.title}: ${info.message.orEmpty()} ${info.errors}", "done", info.state)
        return info
    }

    suspend fun listing(location: String, showHidden: Boolean = true, filter: Any? = null, sortKey: String = "name"): Listing =
        get(
            "fs.list",
            args(
                "location" to location,
                "showHidden" to showHidden,
                "filter" to filter,
                "sort" to mapOf("key" to sortKey, "desc" to false, "dirsFirst" to true),
                "refresh" to true,
            ),
        )

    suspend fun names(location: String): Set<String> = listing(location).entries.map { it.name }.toSet()

    suspend fun child(location: String, name: String): Entry =
        listing(location).entries.firstOrNull { it.name == name }
            ?: throw AssertionError("\"$name\" fehlt in $location: ${names(location)}")

    suspend fun mkdir(parent: String, name: String): Entry = get("fs.mkdir", args("parent" to parent, "name" to name))

    /** Copies [sources] into [targetDir] and waits (local, upload, download). */
    suspend fun copy(sources: List<String>, targetDir: String, conflict: String = "keepBoth"): TaskInfo =
        runTask("fs.transfer", args("sources" to sources, "targetDir" to targetDir, "mode" to "copy", "conflict" to conflict))
}

fun TaskInfo.resultObj(): JsonObject = result as? JsonObject ?: throw AssertionError("Task $id ($kind) ohne Ergebnis: $this")

/** Polls [condition] every 250 ms until it holds; fails with [what] after [timeoutMs]. */
suspend fun waitFor(what: String, timeoutMs: Long = 30_000, condition: suspend () -> Boolean) {
    val deadline = System.currentTimeMillis() + timeoutMs
    while (!condition()) {
        if (System.currentTimeMillis() > deadline) throw AssertionError("Zeitüberschreitung (${timeoutMs / 1000} s): $what")
        delay(250)
    }
}

/** The app's own posted notification with [id], or null. */
fun activeNotification(id: Int): android.service.notification.StatusBarNotification? =
    appContext.getSystemService(android.app.NotificationManager::class.java)
        ?.activeNotifications
        ?.firstOrNull { it.id == id && it.packageName == appContext.packageName }

/** Runs a suite step on the test thread with an overall bound; waits for the core first. */
fun coreTest(timeoutMs: Long = 15 * 60_000L, block: suspend () -> Unit): Unit = runBlocking {
    withTimeout(timeoutMs) {
        Api.awaitReady()
        block()
    }
}

/** Mounted volumes as the app reports them to the core. */
object Volumes {
    fun all(): List<VolumeInfo> = Storage.volumes(appContext)

    fun primary(): VolumeInfo = all().firstOrNull { it.primary } ?: throw AssertionError("Kein primäres Volume: ${all()}")

    fun sdCard(): VolumeInfo =
        all().firstOrNull { it.removable && !it.primary }
            ?: throw AssertionError("Keine SD-Karte gemeldet (Emulator mit sdcard-path-or-size starten): ${all()}")

    fun trashDir(volume: VolumeInfo): File = File(volume.path, ".SmartExplorer-Papierkorb")
}

/** Test folders below `<volume>/SmartExplorerTask/…`, recreated for every test. */
object Fixture {
    const val FOLDER = "SmartExplorerTask"

    fun dir(volume: VolumeInfo, vararg parts: String): File {
        val dir = parts.fold(File(volume.path, FOLDER)) { parent, part -> File(parent, part) }
        if (dir.exists()) dir.deleteRecursively()
        assertTrue("Ordner nicht angelegt: $dir", dir.mkdirs())
        return dir
    }

    fun write(file: File, text: String): File {
        file.parentFile?.mkdirs()
        file.writeText(text)
        return file
    }

    /** Deterministic content of [size] bytes. */
    fun bytes(file: File, size: Int, seed: Int): File {
        file.parentFile?.mkdirs()
        file.outputStream().buffered().use { out ->
            var x = seed
            repeat(size) {
                x = x * 1103515245 + 12345
                out.write(x ushr 16)
            }
        }
        return file
    }

    /** [megabytes] MiB of seeded pseudo-random content (fast, for long transfers). */
    fun big(file: File, megabytes: Int, seed: Long): File {
        file.parentFile?.mkdirs()
        val random = java.util.Random(seed)
        val chunk = ByteArray(1024 * 1024)
        file.outputStream().use { out ->
            repeat(megabytes) {
                random.nextBytes(chunk)
                out.write(chunk)
            }
        }
        return file
    }

    fun sha256(file: File): String {
        val digest = MessageDigest.getInstance("SHA-256")
        file.inputStream().use { input ->
            val buffer = ByteArray(64 * 1024)
            while (true) {
                val read = input.read(buffer)
                if (read < 0) break
                digest.update(buffer, 0, read)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it.toInt() and 0xff) }
    }

    /** Unique suffix for remote folder names (a run never reuses a folder). */
    fun unique(prefix: String): String = "$prefix-${System.currentTimeMillis()}"
}

/** Saved test connections to the servers the host script runs (reachable as 10.0.2.2). */
object Servers {
    val host: String get() = TaskArgs.get("seServerHost")

    fun sftpInput(root: String = TaskArgs.get("seSftpRoot"), password: String = TaskArgs.get("seSftpPass")): JsonObject = args(
        "label" to "Task SFTP",
        "protocol" to "sftp",
        "host" to host,
        "port" to TaskArgs.int("seSftpPort"),
        "user" to TaskArgs.get("seSftpUser"),
        "root" to root,
        "auth" to "password",
        "useAgent" to false,
        "https" to false,
        "password" to password,
    )

    fun ftpInput(): JsonObject = args(
        "label" to "Task FTP",
        "protocol" to "ftp",
        "host" to host,
        "port" to TaskArgs.int("seFtpPort"),
        "user" to TaskArgs.get("seFtpUser"),
        "root" to TaskArgs.get("seFtpRoot"),
        "auth" to "password",
        "useAgent" to false,
        "https" to false,
        "password" to TaskArgs.get("seFtpPass"),
    )

    /** The saved SFTP connection (created on first use). */
    suspend fun sftp(): JsonObject = ensure(sftpInput())

    suspend fun ftp(): JsonObject = ensure(ftpInput())

    private suspend fun ensure(input: JsonObject): JsonObject {
        val existing = Api.objects("conn.list").firstOrNull { connection ->
            connection.text("protocol") == input.text("protocol") &&
                connection.int("port") == input.int("port") &&
                connection.text("user") == input.text("user") &&
                connection.text("root") == input.text("root")
        }
        return existing ?: Api.obj("conn.save", args("input" to input))
    }

    /** A new, empty folder on [connection] (its location). */
    suspend fun freshDir(connection: JsonObject, prefix: String): String =
        Api.mkdir(connection.text("location"), Fixture.unique(prefix)).location
}
