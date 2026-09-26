package app.smartexplorer.android.ui.connections

import app.smartexplorer.android.api.Connection
import app.smartexplorer.android.api.ConnectionInput

/** Form fields that can show an error. */
internal enum class ConnField { Host, Port, KeyPath }

/**
 * Editable connection without secrets (spec F12). Passwords live only in the form state in memory
 * and are passed to [toInput]; they never enter saved UI state.
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
) {
    val isSftp: Boolean
        get() = protocol == "sftp"

    val usesKey: Boolean
        get() = isSftp && auth == "key"

    /** Switching the type keeps a custom port and replaces the previous default. */
    fun withProtocol(value: String): ConnectionDraft = copy(protocol = value, port = nextPort(defaultPort(value, https)))

    fun withHttps(value: Boolean): ConnectionDraft = copy(https = value, port = nextPort(defaultPort(protocol, value)))

    fun errors(): Map<ConnField, String> = buildMap {
        if (host.isBlank()) put(ConnField.Host, "Host fehlt")
        if (portNumber() == null) put(ConnField.Port, "Port zwischen 1 und 65535")
        if (usesKey && keyPath.isBlank()) put(ConnField.KeyPath, "Schlüsseldatei wählen")
    }

    /**
     * Contract input (api.md §4.6). An empty [password] is not sent: for an existing connection the
     * stored one stays, for a new one there is none (e.g. anonymous FTP).
     */
    fun toInput(password: String, passphrase: String): ConnectionInput = ConnectionInput(
        id = id,
        label = label.trim().ifBlank { host.trim() },
        protocol = protocol,
        host = host.trim(),
        port = portNumber() ?: defaultPort(protocol, https),
        user = user.trim(),
        root = root.trim().ifBlank { "/" },
        auth = if (usesKey) "key" else "password",
        keyPath = keyPath.trim().takeIf { usesKey && it.isNotEmpty() },
        useAgent = isSftp && useAgent,
        https = protocol == "webdav" && https,
        password = password.takeIf { it.isNotEmpty() && !usesKey },
        passphrase = passphrase.takeIf { it.isNotEmpty() && usesKey },
    )

    private fun portNumber(): Int? = port.trim().toIntOrNull()?.takeIf { it in 1..65535 }

    private fun nextPort(newDefault: Int): String {
        val current = port.trim()
        return if (current.isEmpty() || current == defaultPort(protocol, https).toString()) newDefault.toString() else current
    }

    companion object {
        /** SFTP 22, FTP/FTPS 21 (explicit TLS), WebDAV 443 or 80. */
        fun defaultPort(protocol: String, https: Boolean): Int = when (protocol) {
            "sftp" -> 22
            "ftp", "ftps" -> 21
            else -> if (https) 443 else 80
        }

        fun from(connection: Connection): ConnectionDraft = ConnectionDraft(
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

        fun protocolLabel(protocol: String): String = when (protocol) {
            "sftp" -> "SFTP"
            "ftp" -> "FTP"
            "ftps" -> "FTPS"
            "webdav" -> "WebDAV"
            else -> protocol.uppercase()
        }
    }
}
