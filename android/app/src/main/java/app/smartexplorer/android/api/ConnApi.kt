package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

// Answers of api.md §4.6 (connections, Google Drive); field names exactly as the contract.

/** A saved connection; [location] is the place to browse it. */
@Serializable
data class Connection(
    val id: String,
    val label: String = "",
    /** `sftp|ftp|ftps|webdav` */
    val protocol: String = "sftp",
    val host: String = "",
    val port: Int = 0,
    val user: String = "",
    val root: String = "",
    /** `password|key` */
    val auth: String = "password",
    val keyPath: String? = null,
    val useAgent: Boolean = false,
    val https: Boolean = false,
    val location: String = "",
)

/**
 * `ConnectionInput`: a [Connection] without id/location plus secrets. With [id] set it edits that
 * connection; a `null` [password] keeps the stored one. `null` fields are not sent.
 */
@Serializable
data class ConnectionInput(
    val id: String? = null,
    val label: String,
    val protocol: String,
    val host: String,
    val port: Int,
    val user: String,
    val root: String,
    val auth: String,
    val keyPath: String? = null,
    val useAgent: Boolean = false,
    val https: Boolean = false,
    val password: String? = null,
    val passphrase: String? = null,
)

/**
 * Cleanup report after removing a connection, a Share device or a room: favorites removed, sync
 * jobs that still use the place (reported only, like the desktop).
 */
@Serializable
data class EndpointRemoval(
    val removedFavorites: Int = 0,
    val orphanedJobs: List<String> = emptyList(),
)

/** `gdrive.status` */
@Serializable
data class GdriveStatus(
    val clientConfigured: Boolean = false,
    val signedIn: Boolean = false,
    val clientId: String? = null,
)

/** Suspending wrappers for api.md §4.6; every call throws [CoreException] on errors. */
object ConnApi {
    val PROTOCOLS = listOf("sftp", "ftp", "ftps", "webdav")

    suspend fun list(): List<Connection> = Core.request<List<Connection>>("conn.list")

    /** Returns the success text; failures throw with kind `auth`, `network`, … */
    suspend fun test(input: ConnectionInput): String = Core.request<Message>("conn.test", inputArgs(input)).message

    suspend fun save(input: ConnectionInput): Connection = Core.request<Connection>("conn.save", inputArgs(input))

    suspend fun delete(id: String): EndpointRemoval =
        Core.request<EndpointRemoval>("conn.delete", buildJsonObject { put("id", id) })

    /** Removes the stored SFTP host key; the next connect trusts the new key (TOFU). */
    suspend fun forgetHostKey(id: String) {
        Core.call("conn.forgetHostKey", buildJsonObject { put("id", id) })
    }

    suspend fun gdriveStatus(): GdriveStatus = Core.request<GdriveStatus>("gdrive.status")

    suspend fun gdriveConfigure(clientId: String, clientSecret: String?) {
        Core.call(
            "gdrive.configure",
            buildJsonObject {
                put("clientId", clientId)
                put("clientSecret", clientSecret)
            },
        )
    }

    /** Starts the browser sign-in; the task ends after the answer or 180 s. */
    suspend fun gdriveSignIn(): String = Core.request<TaskId>("gdrive.signIn").taskId

    suspend fun gdriveSignOut() {
        Core.call("gdrive.signOut")
    }

    private fun inputArgs(input: ConnectionInput): JsonObject = buildJsonObject {
        put("input", Core.json.encodeToJsonElement(ConnectionInput.serializer(), input))
    }

    @Serializable
    private data class Message(val message: String = "")

    @Serializable
    private data class TaskId(val taskId: String)
}
