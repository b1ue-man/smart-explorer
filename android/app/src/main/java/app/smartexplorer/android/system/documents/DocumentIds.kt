package app.smartexplorer.android.system.documents

import android.content.Context

/**
 * Document IDs of the provider (DRV-M8, strand S2 C3), the same rules as
 * `native/src/mobile/os/shared/domains/docs_ids.rs`: a root is `devices`, `conn:<account hex>`,
 * `gdrive` or `vault:<site id>`; a document is the root ID, `/` and the path below the root.
 * Devices are addressed by their key (`devices/direct~<contact>/…`), which a rename leaves alone,
 * so granted URIs stay valid. Everything here is pure: `isChildDocument` needs no I/O.
 */
object DocumentIds {
    const val DEVICES = "devices"
    const val GDRIVE = "gdrive"
    private const val CONN_PREFIX = "conn:"
    private const val VAULT_PREFIX = "vault:"
    private val SITE_ID = Regex("[0-9a-f]{32}")
    private val HEX = Regex("([0-9a-f]{2})+")
    private val DEVICE_KEY = Regex("[A-Za-z0-9._~%-]+")

    fun authority(context: Context): String = "${context.packageName}.documents"

    /** The root of [id], or null for an invalid ID. */
    fun rootOf(id: String): String? = id.substringBefore('/').takeIf { validRoot(it) && valid(id) }

    fun validRoot(root: String): Boolean = when {
        root == DEVICES || root == GDRIVE -> true
        root.startsWith(CONN_PREFIX) -> HEX.matches(root.removePrefix(CONN_PREFIX))
        root.startsWith(VAULT_PREFIX) -> SITE_ID.matches(root.removePrefix(VAULT_PREFIX))
        else -> false
    }

    /** Posix names only: not empty, not `.` or `..`, no `/`, `\` or NUL; device keys checked. */
    fun valid(id: String): Boolean {
        val root = id.substringBefore('/')
        if (!validRoot(root)) return false
        if (!id.contains('/')) return true
        val names = id.substringAfter('/').split('/')
        if (names.any { it.isEmpty() || it == "." || it == ".." || it.contains('\\') || it.contains('\u0000') }) return false
        return root != DEVICES || DEVICE_KEY.matches(names.first())
    }

    /** `isChildDocument`: [child] lies inside [parent] or is it, by the IDs alone. */
    fun isChild(parent: String, child: String): Boolean {
        if (!valid(parent) || !valid(child)) return false
        if (parent.substringBefore('/') != child.substringBefore('/')) return false
        return !parent.contains('/') || child == parent || child.startsWith("$parent/")
    }

    /** The folder [id] is in; null for a root. */
    fun parent(id: String): String? = if (id.contains('/')) id.substringBeforeLast('/') else null

    /**
     * The open mode of `openDocument`: `r`, `w`, `wt`, `rw`, `rwt`. Appending (`wa`, `rwa`) is
     * refused for good: the platform drops the append flag and AppFuse knows no `SETATTR`, so the
     * data would land at offset 0 (NR N20). Returns null for every refused mode.
     */
    fun openMode(mode: String): OpenMode? = when (mode) {
        "r" -> OpenMode(write = false, truncate = false)
        "w", "rw" -> OpenMode(write = true, truncate = false)
        "wt", "rwt" -> OpenMode(write = true, truncate = true)
        else -> null
    }

    data class OpenMode(val write: Boolean, val truncate: Boolean)
}
