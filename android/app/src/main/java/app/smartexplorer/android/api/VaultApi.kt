package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.TaskInfo
import kotlinx.serialization.KSerializer
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

// Vaults on Android (plan 2026-10-17, VLT-M11, festlegungen § U31 with the answers S1–S8 of the
// security agent). Field names exactly as `native/src/mobile/os/shared/domains/vault*.rs`.
// Passwords travel only as arguments of a single call and are never kept here (U31-14).

/** One entered site (`vault.list`, `vault.show`). */
@Serializable
data class VaultSite(
    val site: String,
    val label: String = "",
    val location: String = "",
    /** `local`, `connection`, `share` or `gdrive`. */
    val kind: String = "",
    val connection: String? = null,
    /** `locked`, `unlocked` or `unavailable`. */
    val state: String = "locked",
    val stateDetail: String? = null,
    val replica: Boolean = false,
    val format: Int? = null,
    val cipher: String? = null,
    val rememberPassword: Boolean = false,
    val autoLockIdleMin: Int? = null,
    val lockOnScreenLock: Boolean = true,
    val lockOnBackgroundMin: Int? = 1,
    val syncJobs: List<String> = emptyList(),
    val recoveryKeySupported: Boolean = false,
    val recoveryKeyNote: String? = null,
) {
    val unlocked: Boolean get() = state == "unlocked"
    val unavailable: Boolean get() = state == "unavailable"
}

@Serializable
data class VaultRootBinding(
    val site: String,
    val vault: String,
    val location: String,
    val backend: String,
    val namespace: String,
    val account: String,
    val root: String,
    val `object`: String? = null,
)

@Serializable
data class VaultMountPolicy(
    val binding: VaultRootBinding,
    val enforced: Boolean,
    val trusted: Boolean,
    val cleartext_copy: Boolean,
    val read_only: Boolean,
)

@Serializable
data class VaultOpeningPolicy(
    val site: String,
    val storedRootTrust: Boolean = false,
    val storedRootBinding: VaultRootBinding? = null,
    val current: VaultMountPolicy? = null,
    val resolutionError: String? = null,
    val providerEnabled: Boolean = false,
    val providerReleased: Boolean = false,
    val unlocked: Boolean = false,
)

@Serializable
data class VaultGroup(val vaultId: String? = null, val sites: List<VaultSite> = emptyList())

@Serializable
data class VaultList(val vaults: List<VaultGroup> = emptyList())

@Serializable
data class VaultDetection(
    val isVault: Boolean = false,
    val format: Int? = null,
    val keySource: String? = null,
    val vaultId: String? = null,
    val problem: String? = null,
    val name: String = "",
)

@Serializable
data class VaultAdded(val site: String, val label: String = "")

@Serializable
data class VaultUnlocked(val site: String = "", val already: Boolean = false)

@Serializable
data class VaultCreated(val site: String, val label: String = "", val unlocked: Boolean = false, val unlockError: String? = null)

@Serializable
data class VaultPasswordChanged(val backup: String = "", val rememberCleared: Boolean = false)

@Serializable
data class VaultSealed(val sealed: String)

@Serializable
data class VaultSyncCreated(val job: String, val name: String = "", val siteB: String = "", val createdSite: Boolean = false)

/** Head of a comparison (`vault.compare`) or of a check (`vault.check`, [counts] empty). */
@Serializable
data class VaultCompareHead(
    val list: String,
    val job: String = "",
    val site: String = "",
    val label: String = "",
    val siteA: String = "",
    val labelA: String = "",
    val siteB: String = "",
    val labelB: String = "",
    val revision: Long = 0,
    val counts: Map<String, Int> = emptyMap(),
    val total: Int = 0,
    val unavailable: String? = null,
)

@Serializable
data class VaultSide(val exists: Boolean = false, val size: Long? = null, val mtimeMs: Long? = null, val location: String = "")

/** One entry; [path] is a cleartext path (only on screens with `FLAG_SECURE`, S2/S6). */
@Serializable
data class VaultItem(
    val item: String,
    val group: String = "",
    val kind: String = "file",
    val path: String = "",
    val a: VaultSide = VaultSide(),
    val b: VaultSide = VaultSide(),
    val actions: List<String> = emptyList(),
    val finding: String? = null,
    val note: String? = null,
)

@Serializable
data class VaultItemPage(val items: List<VaultItem> = emptyList(), val nextOffset: Int? = null, val total: Int = 0)

@Serializable
data class VaultTextRow(val left: String? = null, val right: String? = null, val equal: Boolean = false)

@Serializable
data class VaultTextPair(
    val a: String = "",
    val b: String = "",
    val rows: List<VaultTextRow> = emptyList(),
    val crlf: Boolean = false,
    val finalNewline: Boolean = false,
)

@Serializable
data class VaultFailure(val item: String = "", val error: String = "")

@Serializable
data class VaultResolved(val item: String? = null, val resolved: Int = 0, val failed: List<VaultFailure> = emptyList())

/** What a failed vault task keeps in its result: the code and the full text (protected screens). */
@Serializable
data class VaultTaskFailure(val code: String = "internal", val message: String = "")

@Serializable
private data class VaultTaskRef(val taskId: String)

object VaultApi {
    suspend fun openPolicy(site: String): VaultOpeningPolicy = Core.request("vault.openPolicy", buildJsonObject { put("site", site) })

    suspend fun confirmRootTrust(site: String, binding: VaultRootBinding): VaultOpeningPolicy = Core.request(
        "vault.confirmRootTrust", buildJsonObject {
            put("site", site)
            put("binding", buildJsonObject {
                put("site", binding.site); put("vault", binding.vault)
                put("location", binding.location); put("backend", binding.backend)
                put("namespace", binding.namespace); put("account", binding.account)
                put("root", binding.root)
                put("object", binding.`object`?.let { JsonPrimitive(it) } ?: JsonNull)
            })
        },
    )

    suspend fun revokeRootTrust(site: String): VaultOpeningPolicy = Core.request("vault.revokeRootTrust", buildJsonObject { put("site", site) })

    suspend fun providerRelease(site: String, enabled: Boolean): VaultOpeningPolicy = Core.request(
        "vault.providerRelease", buildJsonObject { put("site", site); put("enabled", enabled) },
    )

    suspend fun list(): VaultList = Core.request("vault.list")

    suspend fun show(site: String): VaultSite = Core.request("vault.show", buildJsonObject { put("site", site) })

    suspend fun detect(location: String): VaultDetection = Core.request("vault.detect", buildJsonObject { put("location", location) })

    suspend fun add(location: String, label: String?): VaultAdded = Core.request(
        "vault.add",
        buildJsonObject {
            put("location", location)
            label?.takeIf { it.isNotBlank() }?.let { put("label", it) }
        },
    )

    suspend fun remove(site: String) {
        Core.call("vault.remove", buildJsonObject { put("site", site) })
    }

    /** Starts `vault.unlock`; the task ID is for „Abbrechen“ (S4). */
    suspend fun startUnlock(site: String, password: String, memAvail: Long, memThreshold: Long): String =
        Core.request<VaultTaskRef>(
            "vault.unlock",
            buildJsonObject {
                put("site", site)
                put("password", password)
                put("memAvail", memAvail)
                put("memThreshold", memThreshold)
            },
        ).taskId

    suspend fun lock(site: String?, keepUnsaved: Boolean = false) {
        Core.call(
            "vault.lock",
            buildJsonObject {
                site?.let { put("site", it) }
                put("keepUnsaved", keepUnsaved)
            },
        )
    }

    suspend fun lockAll() {
        Core.call("vault.lockAll")
    }

    suspend fun create(location: String, label: String?, password: String, memAvail: Long, memThreshold: Long): VaultCreated =
        runTask(
            "vault.create",
            buildJsonObject {
                put("location", location)
                label?.takeIf { it.isNotBlank() }?.let { put("label", it) }
                put("password", password)
                put("memAvail", memAvail)
                put("memThreshold", memThreshold)
            },
            VaultCreated.serializer(),
        )

    suspend fun changePassword(site: String, oldPassword: String, newPassword: String): VaultPasswordChanged = runTask(
        "vault.changePassword",
        buildJsonObject {
            put("site", site)
            put("oldPassword", oldPassword)
            put("newPassword", newPassword)
        },
        VaultPasswordChanged.serializer(),
    )

    suspend fun forgetPassword(site: String) {
        Core.call("vault.forgetPassword", buildJsonObject { put("site", site) })
    }

    /** Stores the envelope sealed after the device lock confirmed it (S1). */
    suspend fun rememberSaved(site: String, sealed: String) {
        Core.call(
            "vault.rememberSaved",
            buildJsonObject {
                put("site", site)
                put("sealed", sealed)
            },
        )
    }

    suspend fun savedSecret(site: String): String = Core.request<VaultSealed>("vault.savedSecret", buildJsonObject { put("site", site) }).sealed

    /** Changes only the given settings; `null` minutes switch the timed lock off ([setIdle]/[setBackground]). */
    suspend fun setSettings(
        site: String,
        label: String? = null,
        setIdle: Boolean = false,
        autoLockIdleMin: Int? = null,
        lockOnScreenLock: Boolean? = null,
        setBackground: Boolean = false,
        lockOnBackgroundMin: Int? = null,
    ): VaultSite = Core.request(
        "vault.setSettings",
        buildJsonObject {
            put("site", site)
            label?.let { put("label", it) }
            if (setIdle) put("autoLockIdleMin", autoLockIdleMin?.let { JsonPrimitive(it) } ?: JsonNull)
            lockOnScreenLock?.let { put("lockOnScreenLock", it) }
            if (setBackground) put("lockOnBackgroundMin", lockOnBackgroundMin?.let { JsonPrimitive(it) } ?: JsonNull)
        },
    )

    /** [target] `replica`, `site` or `location`; [with] the site or location for the last two. */
    suspend fun syncCreate(site: String, target: String, with: String?, name: String?, direction: String): VaultSyncCreated = Core.request(
        "vault.syncCreate",
        buildJsonObject {
            put("site", site)
            put("target", target)
            with?.let { put("with", it) }
            name?.takeIf { it.isNotBlank() }?.let { put("name", it) }
            put("direction", direction)
        },
    )

    suspend fun compare(job: String): VaultCompareHead = runTask(
        "vault.compare",
        buildJsonObject {
            put("job", job)
            put("groups", JsonArray(GROUPS.map { JsonPrimitive(it) }))
        },
        VaultCompareHead.serializer(),
    )

    suspend fun check(site: String): VaultCompareHead =
        runTask("vault.check", buildJsonObject { put("site", site) }, VaultCompareHead.serializer())

    suspend fun items(list: String, revision: Long, group: String?, offset: Int, limit: Int): VaultItemPage = Core.request(
        "vault.compareItems",
        buildJsonObject {
            put("list", list)
            put("revision", revision)
            group?.let { put("group", it) }
            put("offset", offset)
            put("limit", limit)
        },
    )

    suspend fun compareText(job: String, item: String): VaultTextPair = Core.request(
        "vault.compareText",
        buildJsonObject {
            put("job", job)
            put("item", item)
        },
    )

    suspend fun resolve(job: String, item: String, action: String, confirmed: Boolean): VaultResolved = runTask(
        "vault.resolve",
        buildJsonObject {
            put("job", job)
            put("item", item)
            put("action", action)
            put("confirmed", confirmed)
        },
        VaultResolved.serializer(),
    )

    suspend fun resolveAll(job: String, action: String): VaultResolved = runTask(
        "vault.resolveAll",
        buildJsonObject {
            put("job", job)
            put("action", action)
            put("confirmed", true)
        },
        VaultResolved.serializer(),
    )

    suspend fun mergeApply(job: String, item: String, content: String): VaultResolved = runTask(
        "vault.mergeApply",
        buildJsonObject {
            put("job", job)
            put("item", item)
            put("content", content)
        },
        VaultResolved.serializer(),
    )

    /** Waits for the task [taskId] and decodes its result; a failed task throws its code (U31-11). */
    suspend fun <T> awaitTask(taskId: String, serializer: KSerializer<T>): T = resultOrThrow(FilesApi.awaitTask(taskId), serializer)

    suspend fun cancelTask(taskId: String) {
        Core.call("task.cancel", buildJsonObject { put("id", taskId) })
    }

    internal fun <T> resultOrThrow(task: TaskInfo, serializer: KSerializer<T>): T {
        if (task.state == "done") {
            return FilesApi.resultOf(task, serializer) ?: throw CoreException("internal", "Ergebnis von ${task.kind} fehlt")
        }
        val failure = try {
            FilesApi.resultOf(task, VaultTaskFailure.serializer())
        } catch (e: CoreException) {
            null
        }
        if (task.state == "canceled") throw CoreException("canceled", failure?.message ?: "Abgebrochen")
        throw CoreException(failure?.code ?: "internal", failure?.message ?: task.message ?: "Fehlgeschlagen")
    }

    private suspend fun <T> runTask(method: String, args: JsonObject, serializer: KSerializer<T>): T =
        awaitTask(Core.request<VaultTaskRef>(method, args).taskId, serializer)

    /** Every group of a comparison, in the order of the screen (U31-8). */
    val GROUPS = listOf("conflict", "changed_a", "changed_b", "new_a", "new_b", "deleted_a", "deleted_b", "deferred", "structure")
}
