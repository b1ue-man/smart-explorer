package app.smartexplorer.android.ui.connections

import app.smartexplorer.android.api.Connection
import app.smartexplorer.android.api.ConnectionInput

/** Form fields that can show an error. */
internal enum class ConnField { Host, Port, KeyPath, Share }

/**
 * Editable connection without secrets (spec F12). Passwords live only in the form state in memory
 * and are passed to [toInput]; they never enter saved UI state.
 *
 * SMB (api.md §4.6): [share] and [domain] exist only in the form. The contract stores them as
 * `root = /<freigabe>/<startordner>` and `user = DOMÄNE\benutzer`; for SMB [root] is the start
 * folder inside the share.
 */
internal data class ConnectionDraft(
    val id: String? = null,
    val protocol: String = "sftp",
    val label: String = "",
    val host: String = "",
    val port: String = "22",
    val user: String = "",
    val root: String = "/",
    /** `password|key` (SFTP only; other protocols always use a password). */
    val auth: String = "password",
    val keyPath: String = "",
    val useAgent: Boolean = false,
    val https: Boolean = true,
    /** SMB share (form only). */
    val share: String = "",
    /** Optional SMB domain (form only). */
    val domain: String = "",
) {
    val isSftp: Boolean
        get() = protocol == "sftp"

    val isSmb: Boolean
        get() = protocol == "smb"

    val usesKey: Boolean
        get() = isSftp && auth == "key"

    /** Switching the type keeps a custom port and replaces the previous default. */
    fun withProtocol(value: String): ConnectionDraft = copy(protocol = value, port = nextPort(defaultPort(value, https)))

    fun withHttps(value: Boolean): ConnectionDraft = copy(https = value, port = nextPort(defaultPort(protocol, value)))

    /**
     * Host field input. A pasted `\\host\freigabe\pfad` or `smb://[user@]host[:port]/freigabe/pfad`
     * selects SMB and fills host, port, user, share and start folder; anything else is the host.
     */
    fun withHostInput(value: String): ConnectionDraft {
        val address = SmbAddress.parse(value) ?: return copy(host = value)
        val smb = if (isSmb) this else withProtocol("smb")
        return smb.copy(
            host = address.host,
            port = address.port?.toString() ?: smb.port,
            user = address.user ?: smb.user,
            share = address.share ?: smb.share,
            root = if (address.share != null) address.path else smb.root,
        )
    }

    fun errors(): Map<ConnField, String> {
        val draft = resolved()
        return buildMap {
            if (draft.host.isBlank()) put(ConnField.Host, "Host fehlt")
            if (draft.portNumber() == null) put(ConnField.Port, "Port zwischen 1 und 65535")
            if (usesKey && keyPath.isBlank()) put(ConnField.KeyPath, "Schlüsseldatei wählen")
            if (isSmb) {
                val share = draft.share.trim()
                if (share.isEmpty()) {
                    put(ConnField.Share, "Freigabe fehlt")
                } else if (share.any { it == '/' || it == '\\' }) {
                    put(ConnField.Share, "Nur der Name der Freigabe")
                }
            }
        }
    }

    /**
     * Contract input (api.md §4.6). An empty [password] is not sent: for an existing connection the
     * stored one stays, for a new one there is none (e.g. anonymous FTP).
     */
    fun toInput(password: String, passphrase: String): ConnectionInput {
        val draft = resolved()
        return ConnectionInput(
            id = id,
            label = draft.label.trim().ifBlank { draft.host.trim() },
            protocol = protocol,
            host = draft.host.trim(),
            port = draft.portNumber() ?: defaultPort(protocol, https),
            user = draft.inputUser(),
            root = draft.inputRoot(),
            auth = if (usesKey) "key" else "password",
            keyPath = keyPath.trim().takeIf { usesKey && it.isNotEmpty() },
            useAgent = isSftp && useAgent,
            https = protocol == "webdav" && https,
            password = password.takeIf { it.isNotEmpty() && !usesKey },
            passphrase = passphrase.takeIf { it.isNotEmpty() && usesKey },
        )
    }

    /** An SMB address typed (not pasted) into the host field is split as well. */
    private fun resolved(): ConnectionDraft =
        if (isSmb && SmbAddress.parse(host) != null) withHostInput(host) else this

    private fun inputUser(): String {
        val name = user.trim()
        val prefix = domain.trim()
        return if (isSmb && prefix.isNotEmpty() && '\\' !in name) "$prefix\\$name" else name
    }

    private fun inputRoot(): String {
        val path = root.trim().ifBlank { "/" }
        if (!isSmb) return path
        val folders = path.replace('\\', '/').split('/').filter { it.isNotEmpty() }
        return "/" + (listOf(share.trim()) + folders).joinToString("/")
    }

    private fun portNumber(): Int? = port.trim().toIntOrNull()?.takeIf { it in 1..65535 }

    private fun nextPort(newDefault: Int): String {
        val current = port.trim()
        return if (current.isEmpty() || current == defaultPort(protocol, https).toString()) newDefault.toString() else current
    }

    companion object {
        /** SFTP 22, FTP/FTPS 21 (explicit TLS), SMB 445, WebDAV 443 or 80. */
        fun defaultPort(protocol: String, https: Boolean): Int = when (protocol) {
            "sftp" -> 22
            "ftp", "ftps" -> 21
            "smb" -> 445
            else -> if (https) 443 else 80
        }

        fun from(connection: Connection): ConnectionDraft {
            val draft = ConnectionDraft(
                id = connection.id,
                protocol = connection.protocol,
                label = connection.label,
                host = connection.host,
                port = connection.port.takeIf { it > 0 }?.toString() ?: defaultPort(connection.protocol, connection.https).toString(),
                user = connection.user,
                root = connection.root.ifBlank { "/" },
                auth = connection.auth,
                keyPath = connection.keyPath.orEmpty(),
                useAgent = connection.useAgent,
                // Only a saved WebDAV connection keeps its scheme; any other type switched to WebDAV
                // starts on HTTPS like a new draft (the form has no HTTPS switch).
                https = connection.protocol != "webdav" || connection.https,
            )
            if (connection.protocol != "smb") return draft
            val segments = connection.root.split('/').filter { it.isNotEmpty() }
            val domainUser = connection.user.split('\\', limit = 2)
            return draft.copy(
                share = segments.firstOrNull().orEmpty(),
                root = "/" + segments.drop(1).joinToString("/"),
                domain = if (domainUser.size == 2) domainUser[0] else "",
                user = domainUser.last(),
            )
        }

        fun protocolLabel(protocol: String): String = when (protocol) {
            "sftp" -> "SFTP"
            "ftp" -> "FTP"
            "ftps" -> "FTPS"
            "webdav" -> "WebDAV"
            "smb" -> "SMB"
            else -> protocol.uppercase()
        }
    }
}

/**
 * A pasted SMB address: `\\host\freigabe\pfad` (also with `/`) or
 * `smb://[[domäne;]benutzer[:passwort]@]host[:port][/freigabe[/pfad]]`. A password in the URL is
 * ignored (it never enters the draft); [share] is `null` when the address names none.
 */
internal data class SmbAddress(
    val host: String,
    val port: Int?,
    val user: String?,
    val share: String?,
    /** Start folder inside the share, `/` when none. */
    val path: String,
) {
    companion object {
        fun parse(text: String): SmbAddress? {
            val value = text.trim()
            return when {
                value.startsWith("\\\\") || value.startsWith("//") -> {
                    val parts = value.substring(2).split('\\', '/')
                    val (host, port) = splitHostPort(parts.first()) ?: return null
                    of(host, port, null, parts.drop(1))
                }
                value.startsWith("smb://", ignoreCase = true) -> {
                    val rest = value.substring(6)
                    val authority = rest.substringBefore('/')
                    val at = authority.lastIndexOf('@')
                    val user = if (at < 0) null else authority.substring(0, at).substringBefore(':').replace(';', '\\')
                    val (host, port) = splitHostPort(authority.substring(at + 1)) ?: return null
                    of(host, port, user?.ifEmpty { null }, rest.substringAfter('/', "").split('/'))
                }
                else -> null
            }
        }

        private fun of(host: String, port: Int?, user: String?, segments: List<String>): SmbAddress {
            val names = segments.filter { it.isNotEmpty() }
            return SmbAddress(host, port, user, names.firstOrNull(), "/" + names.drop(1).joinToString("/"))
        }

        /** `host`, `host:port`, `[v6]` or `[v6]:port`; `null` when it is not an address. */
        private fun splitHostPort(value: String): Pair<String, Int?>? {
            val (host, portText) = when {
                value.startsWith("[") -> {
                    val end = value.indexOf(']')
                    if (end < 0) return null
                    value.substring(1, end) to value.substring(end + 1).removePrefix(":").ifEmpty { null }
                }
                value.count { it == ':' } == 1 -> value.substringBefore(':') to value.substringAfter(':')
                else -> value to null
            }
            if (host.isBlank() || host.any { it.isWhitespace() }) return null
            val port = portText?.let { text -> text.toIntOrNull()?.takeIf { it in 1..65535 } ?: return null }
            return host to port
        }
    }
}
