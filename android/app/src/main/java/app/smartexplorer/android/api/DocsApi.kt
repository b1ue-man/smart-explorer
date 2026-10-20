package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.CoreState
import app.smartexplorer.android.core.NativeBridge
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

// Documents provider (plan 2026-10-17, DRV-M8; bridge `docs.*`): what other apps see through
// „Ordner für Apps bereitstellen“. Only released roots exist for them; nothing is released by
// default. Field names exactly as `native/src/mobile/os/shared/domains/docs*.rs`.

/** One root of the system picker (`docs.roots`). */
@Serializable
data class DocsRoot(
    val rootId: String,
    val documentId: String,
    val title: String,
    /** `devices`, `conn`, `gdrive` or `vault`. */
    val kind: String = "",
    /** The root folder takes new entries (`FLAG_SUPPORTS_CREATE`). */
    val create: Boolean = false,
    /** A vault in this phone's storage (`FLAG_LOCAL_ONLY`, assumption A10). */
    val localOnly: Boolean = false,
)

/** A file or folder (`docs.document`, entries of `docs.children`). */
@Serializable
data class DocsItem(
    val documentId: String,
    val name: String,
    val dir: Boolean = false,
    val size: Long = 0,
    /** Milliseconds since 1970; 0 when unknown. */
    val mtime: Long = 0,
    val writable: Boolean = false,
    /** Device folders only: the device answers now. */
    val online: Boolean? = null,
)

/** `docs.children`: [loading] until `docs.changed` names the folder; [error] for the picker. */
@Serializable
data class DocsChildren(
    val loading: Boolean = false,
    val error: String? = null,
    val children: List<DocsItem> = emptyList(),
)

/** A connection or vault that can be released. */
@Serializable
data class DocsChoice(
    val id: String,
    val label: String,
    val unlocked: Boolean = false,
    val local: Boolean = false,
)

@Serializable
data class DocsChoices(
    val connections: List<DocsChoice> = emptyList(),
    val vaults: List<DocsChoice> = emptyList(),
)

/** `docs.settings.get`: [connections] holds account keys and `gdrive`, [vaults] site IDs. */
@Serializable
data class DocsSettings(
    val provider: Boolean = false,
    val devices: Boolean = false,
    val connections: List<String> = emptyList(),
    val vaults: List<String> = emptyList(),
    val choices: DocsChoices = DocsChoices(),
)

/** `docs.pending` / `docs.retryPending`. */
@Serializable
data class DocsPending(val pending: Int = 0, val failed: List<String> = emptyList())

/** Created or renamed document (`docs.rename`). */
@Serializable
data class DocsId(val documentId: String)

/**
 * What the provider needs from the core. [NativeDocsBackend] is the real one; the Robolectric
 * tests pass their own. Errors of the metadata calls are [CoreException]; the descriptor calls
 * return a negated errno.
 */
interface DocsBackend {
    fun roots(): List<DocsRoot>
    fun document(id: String): DocsItem
    fun children(id: String): DocsChildren
    fun create(parentId: String, name: String, dir: Boolean): DocsItem
    fun rename(id: String, name: String): String
    fun delete(id: String)
    fun open(id: String, mode: String): Long
    fun read(handle: Long, offset: Long, size: Int, data: ByteArray): Int
    fun write(handle: Long, offset: Long, size: Int, data: ByteArray): Int
    fun size(handle: Long): Long
    fun fsync(handle: Long): Int
    fun release(handle: Long): Int
}

/** The provider's calls into the core: blocking, as `ContentProvider` calls are. */
class NativeDocsBackend(private val timeoutMs: Long = CALL_TIMEOUT_MS) : DocsBackend {
    private inline fun <reified T> blocking(method: String, crossinline args: kotlinx.serialization.json.JsonObjectBuilder.() -> Unit = {}): T =
        runBlocking { withTimeout(timeoutMs) { Core.request<T>(method, buildJsonObject(args)) } }

    // DocumentsUI waits 5 s for the roots; they never need the network.
    override fun roots(): List<DocsRoot> =
        runBlocking { withTimeout(ROOTS_TIMEOUT_MS) { Core.request<List<DocsRoot>>("docs.roots") } }

    override fun document(id: String): DocsItem = blocking("docs.document") { put("documentId", id) }

    override fun children(id: String): DocsChildren = blocking("docs.children") { put("documentId", id) }

    override fun create(parentId: String, name: String, dir: Boolean): DocsItem = blocking("docs.create") {
        put("documentId", parentId)
        put("name", name)
        put("dir", dir)
    }

    override fun rename(id: String, name: String): String = blocking<DocsId>("docs.rename") {
        put("documentId", id)
        put("name", name)
    }.documentId

    override fun delete(id: String) {
        runBlocking { withTimeout(timeoutMs) { Core.call("docs.delete", buildJsonObject { put("documentId", id) }) } }
    }

    override fun open(id: String, mode: String): Long {
        // The library is loaded by the core start; a cold provider waits for it.
        val state = runBlocking { withTimeout(timeoutMs) { Core.ready.first { it !is CoreState.Starting } } }
        if (state is CoreState.Failed) return -EIO
        return NativeBridge.docOpen(id, mode)
    }

    override fun read(handle: Long, offset: Long, size: Int, data: ByteArray): Int = NativeBridge.docRead(handle, offset, size, data)

    override fun write(handle: Long, offset: Long, size: Int, data: ByteArray): Int = NativeBridge.docWrite(handle, offset, size, data)

    override fun size(handle: Long): Long = NativeBridge.docSize(handle)

    override fun fsync(handle: Long): Int = NativeBridge.docFsync(handle)

    override fun release(handle: Long): Int = NativeBridge.docRelease(handle)

    companion object {
        private const val EIO = 5L
        const val CALL_TIMEOUT_MS = 120_000L
        const val ROOTS_TIMEOUT_MS = 4_000L
    }
}

/** The settings section and the commit worker. */
object DocsApi {
    suspend fun settings(): DocsSettings = Core.request("docs.settings.get")

    /** Changes only the given fields; returns the new settings. */
    suspend fun setSettings(
        provider: Boolean? = null,
        devices: Boolean? = null,
        connections: List<String>? = null,
        vaults: List<String>? = null,
    ): DocsSettings = Core.request(
        "docs.settings.set",
        buildJsonObject {
            provider?.let { put("provider", it) }
            devices?.let { put("devices", it) }
            connections?.let { list -> put("connections", JsonArray(list.map { JsonPrimitive(it) })) }
            vaults?.let { list -> put("vaults", JsonArray(list.map { JsonPrimitive(it) })) }
        },
    )

    suspend fun pending(): DocsPending = Core.request("docs.pending")

    /** Waits for running uploads, then retries what waits (also after a process end). */
    suspend fun retryPending(): DocsPending = Core.request("docs.retryPending")
}
