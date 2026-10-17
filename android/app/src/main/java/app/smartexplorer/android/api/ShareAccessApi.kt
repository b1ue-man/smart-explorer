package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

// Access rights of this phone as exec host (plan 2026-10-17, PRM-M10; api.md §5): the fields
// `execProvider.accountChoices`, `otherAccountsNote`, `elevated`, `capabilities` and
// `execTargets[].access` of `share.status`, `share.setAccess`, `share.accessApprovals`,
// `share.decideAccessApproval` and `share.reportApprovalUndeliverable`; field names exactly as the
// contract. Android runs commands only as the app's own account.

/** Decisions of one capability. */
const val ACCESS_OFF = "off"
const val ACCESS_ASK = "ask"
const val ACCESS_ALLOW = "allow"

/** Capability keys of the bridge; screen keys only come with SCR-M8. */
const val CAP_COMMANDS = "execCommands"
const val CAP_TERMINAL = "execTerminal"
const val CAP_SCREEN_VIEW = "screenView"
const val CAP_SCREEN_CONTROL = "screenControl"

/** Answers to a question of another device: „Diesmal erlauben“, „Immer erlauben“, „Ablehnen“. */
const val DECISION_ONCE = "once"
const val DECISION_ALWAYS = "always"
const val DECISION_DENY = "deny"

/** The account commands run as: `own` (the app's), `remote` or `account` with [name]. */
@Serializable
data class AccessAccount(val kind: String = "own", val name: String? = null)

/** `execTargets[].access`: account, decision per capability key and end (Unix seconds). */
@Serializable
data class AccessView(
    val account: AccessAccount = AccessAccount(),
    val accountLabel: String = "",
    val capabilities: Map<String, String> = emptyMap(),
    val validUntil: Long? = null,
    val expired: Boolean = false,
) {
    /** The decision for [key]; a missing key is off. */
    fun decision(key: String): String = capabilities[key] ?: ACCESS_OFF

    /** A start of one of the Exec capabilities waits for this phone's user. */
    fun asks(): Boolean = decision(CAP_COMMANDS) == ACCESS_ASK || decision(CAP_TERMINAL) == ACCESS_ASK
}

/** One account a right can name here (on Android only the app's own). */
@Serializable
data class AccountChoiceInfo(
    val profile: String = "own",
    val label: String = "",
    val account: String? = null,
    val available: Boolean = false,
    val reason: String? = null,
    val nextStep: String? = null,
    val elevated: Boolean = false,
    val adminGroup: Boolean = false,
)

/** One row of the access dialog: capability [key], label and what allowing it means. */
@Serializable
data class CapabilityRow(
    val key: String,
    val label: String = "",
    val description: String = "",
    val usesAccount: Boolean = false,
)

/** The access part of `share.status.execProvider`. */
@Serializable
data class AccessProviderInfo(
    val userLabel: String = "",
    val accountChoices: List<AccountChoiceInfo> = emptyList(),
    /** Why only the app's own account runs commands on Android (plan C4, word for word). */
    val otherAccountsNote: String = "",
    val elevated: Boolean = false,
    val capabilities: List<CapabilityRow> = emptyList(),
)

@Serializable
data class AccessTargetInfo(val targetKey: String, val access: AccessView = AccessView())

/** The access part of `share.status`: provider fields and the right of every target. */
@Serializable
data class AccessOverview(
    val execProvider: AccessProviderInfo = AccessProviderInfo(),
    val execTargets: List<AccessTargetInfo> = emptyList(),
) {
    fun accessOf(targetKey: String): AccessView? = execTargets.firstOrNull { it.targetKey == targetKey }?.access
}

/** The end in a change: [None] removes it, [At] sets it (Unix seconds). */
sealed interface ValidUntil {
    data object None : ValidUntil

    data class At(val seconds: Long) : ValidUntil
}

/** A change of `share.setAccess`; every `null` field keeps its value. */
data class AccessChange(
    val account: AccessAccount? = null,
    val capabilities: Map<String, String>? = null,
    val programs: List<String>? = null,
    val validUntil: ValidUntil? = null,
)

/** `share.setAccess` */
@Serializable
data class AccessSetResult(val revision: Long, val access: AccessView = AccessView())

/** The command of a question: `argv` with [program] and [args], `shell` with [text], or `login`. */
@Serializable
data class ApprovalCommandInfo(
    val mode: String = "",
    val program: String? = null,
    val args: List<String> = emptyList(),
    val text: String? = null,
)

/**
 * A question of another device that waits for this phone's user (`share.accessApprovals`). Texts
 * come from the core, cut and cleaned; [capability] is a bridge key, [command] is `null` for the
 * screen.
 */
@Serializable
data class AccessApproval(
    val id: String,
    val deviceName: String = "",
    val deviceId: String = "",
    val fingerprint: String = "",
    /** `direct` or `room`. */
    val relation: String = EXEC_RELATION_DIRECT,
    val room: String? = null,
    val capability: String = "",
    val capabilityLabel: String = "",
    /** „Laptop möchte Befehle ausführen“. */
    val text: String = "",
    val accountLabel: String? = null,
    val command: ApprovalCommandInfo? = null,
    val requestedAt: Long = 0,
    /** `notification` on Android. */
    val surface: String? = null,
    /** A client older than the access rights asks for a terminal: it waits at most 60 s. */
    val olderClientTerminal: Boolean = false,
) {
    /** Watching or controlling the screen (answered with SCR-M8's consent activity). */
    val screen: Boolean
        get() = capability == CAP_SCREEN_VIEW || capability == CAP_SCREEN_CONTROL
}

@Serializable
data class AccessApprovals(val pending: List<AccessApproval> = emptyList())

/** The arguments of `share.setAccess`; `validUntil` [ValidUntil.None] is sent as `null`. */
internal fun setAccessArgs(targetKey: String, change: AccessChange): JsonObject = buildJsonObject {
    put("targetKey", targetKey)
    change.account?.let { account ->
        put(
            "account",
            buildJsonObject {
                put("kind", account.kind)
                account.name?.let { put("name", it) }
            },
        )
    }
    change.capabilities?.let { decisions ->
        put("capabilities", buildJsonObject { decisions.forEach { (key, decision) -> put(key, decision) } })
    }
    change.programs?.let { list -> put("programs", JsonArray(list.map { JsonPrimitive(it) })) }
    when (val until = change.validUntil) {
        null -> Unit
        ValidUntil.None -> put("validUntil", JsonNull)
        is ValidUntil.At -> put("validUntil", until.seconds)
    }
}

/** Suspending wrappers of the access methods; every call throws `CoreException` on errors. */
object ShareAccessApi {
    /** The access fields of `share.status` (the rest of the status is ignored here). */
    suspend fun overview(): AccessOverview = Core.request<AccessOverview>("share.status")

    /**
     * Changes the right of [targetKey] (`invalid` for a refused value, `unsupported` for another
     * account or a capability this phone does not offer, `not_found` when the identity changed).
     * A change that narrows the right ends the device's running commands.
     */
    suspend fun setAccess(targetKey: String, change: AccessChange): AccessSetResult =
        Core.request<AccessSetResult>("share.setAccess", setAccessArgs(targetKey, change))

    /** Open questions of other devices, oldest first. */
    suspend fun approvals(): List<AccessApproval> = Core.request<AccessApprovals>("share.accessApprovals").pending

    /** The answer to exactly [approvalId]; `not_found` once the question is no longer open. */
    suspend fun decide(approvalId: String, decision: String) {
        Core.call(
            "share.decideAccessApproval",
            buildJsonObject {
                put("approvalId", approvalId)
                put("decision", decision)
            },
        )
    }

    /** This phone can show [approvalId] nowhere: the other device gets `approval_unavailable`. */
    suspend fun reportUndeliverable(approvalId: String) {
        Core.call("share.reportApprovalUndeliverable", buildJsonObject { put("approvalId", approvalId) })
    }
}
