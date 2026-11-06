package app.smartexplorer.android.ui.share

import app.smartexplorer.android.api.ACCESS_ALLOW
import app.smartexplorer.android.api.ACCESS_ASK
import app.smartexplorer.android.api.ACCESS_OFF
import app.smartexplorer.android.api.AccessOverview
import app.smartexplorer.android.api.AccessProviderInfo
import app.smartexplorer.android.api.CAP_COMMANDS
import app.smartexplorer.android.api.CAP_SCREEN_CONTROL
import app.smartexplorer.android.api.CAP_SCREEN_VIEW
import app.smartexplorer.android.api.CAP_TERMINAL
import app.smartexplorer.android.api.ExecProviderInfo
import app.smartexplorer.android.api.FOLDERS_ALL
import app.smartexplorer.android.api.FOLDERS_NONE
import app.smartexplorer.android.api.FOLDERS_SELECTED
import app.smartexplorer.android.api.PRESET_ALL
import app.smartexplorer.android.api.PRESET_FILES_EXEC
import app.smartexplorer.android.api.PRESET_NONE
import app.smartexplorer.android.api.PRESET_READ_ONLY
import app.smartexplorer.android.api.PRESET_READ_WRITE
import app.smartexplorer.android.api.PendingRights
import app.smartexplorer.android.api.RIGHTS_ACCOUNT_OWN
import app.smartexplorer.android.api.RIGHTS_FIELDS
import app.smartexplorer.android.api.RIGHTS_FIELD_ACCOUNT
import app.smartexplorer.android.api.RIGHTS_FIELD_COMMANDS
import app.smartexplorer.android.api.RIGHTS_FIELD_FOLDERS
import app.smartexplorer.android.api.RIGHTS_FIELD_SCREEN_CONTROL
import app.smartexplorer.android.api.RIGHTS_FIELD_SCREEN_VIEW
import app.smartexplorer.android.api.RIGHTS_FIELD_TERMINAL
import app.smartexplorer.android.api.RIGHTS_FIELD_VALID_UNTIL
import app.smartexplorer.android.api.RIGHTS_FIELD_WRITE
import app.smartexplorer.android.api.Rights
import app.smartexplorer.android.api.RightsFolders
import app.smartexplorer.android.api.RightsPresetInfo
import app.smartexplorer.android.api.RightsSelection
import app.smartexplorer.android.api.RightsTarget
import app.smartexplorer.android.api.SHARE_DIRECT
import app.smartexplorer.android.api.ShareDevice
import app.smartexplorer.android.api.ShareExport
import app.smartexplorer.android.api.ShareRoom
import app.smartexplorer.android.api.ShareStatus
import app.smartexplorer.android.api.rightsFieldsJson
import app.smartexplorer.android.ui.common.Format

// The compact rights menu of a new connection (plan 2026-10-31 §F, F1–F7, package G3). Everything
// here is plain Kotlin (no Compose), so the presets, the sheet's state and the calls it makes are
// covered by JVM unit tests.

/** The presets of api.md §5.1 as the bridge defines them; used while `share.rightsPresets` is not loaded. */
internal object RightsPresets {
    private fun files(write: Boolean) = Rights(folders = RightsFolders(FOLDERS_ALL), write = write)

    fun builtIn(): List<RightsPresetInfo> = listOf(
        RightsPresetInfo(PRESET_NONE, "Keine Rechte", "Die Verbindung bleibt, sie darf nichts.", Rights(preset = PRESET_NONE)),
        RightsPresetInfo(PRESET_READ_ONLY, "Nur lesen", "Alle freigegebenen Ordner ansehen und kopieren.", files(write = false).copy(preset = PRESET_READ_ONLY)),
        RightsPresetInfo(PRESET_READ_WRITE, "Dateien lesen und schreiben", "Alle freigegebenen Ordner lesen und ändern, auch löschen.", files(write = true).copy(preset = PRESET_READ_WRITE)),
        RightsPresetInfo(
            PRESET_FILES_EXEC,
            "Dateien und Codeausführung",
            "Dateien lesen und schreiben, dazu Befehle und Terminal als App-Konto.",
            files(write = true).copy(commands = ACCESS_ALLOW, terminal = ACCESS_ALLOW, preset = PRESET_FILES_EXEC),
        ),
        RightsPresetInfo(
            PRESET_ALL,
            "Alles",
            "Wie „Dateien und Codeausführung“, dazu den Desktop sehen und steuern.",
            files(write = true).copy(
                commands = ACCESS_ALLOW,
                terminal = ACCESS_ALLOW,
                screenView = ACCESS_ALLOW,
                screenControl = ACCESS_ALLOW,
                preset = PRESET_ALL,
            ),
        ),
    )

    /** The presets of the bridge; the built-in ones when it names none (older bridge or load failure). */
    fun of(loaded: List<RightsPresetInfo>?): List<RightsPresetInfo> = loaded?.takeIf { it.isNotEmpty() } ?: builtIn()

    /** The first preset whose files and decisions equal [rights]'s (the end and the account are not part of a preset). */
    fun matching(rights: Rights, presets: List<RightsPresetInfo>): RightsPresetInfo? =
        presets.firstOrNull { preset -> PRESET_FIELDS.all { sameField(rights, preset.rights, it) } }

    private val PRESET_FIELDS = listOf(
        RIGHTS_FIELD_FOLDERS, RIGHTS_FIELD_WRITE, RIGHTS_FIELD_COMMANDS, RIGHTS_FIELD_TERMINAL,
        RIGHTS_FIELD_SCREEN_VIEW, RIGHTS_FIELD_SCREEN_CONTROL,
    )
}

/** Whether [field] of [a] and [b] are equal; folders by kind and (for a selection) by their set of paths. */
internal fun sameField(a: Rights, b: Rights, field: String): Boolean = when (field) {
    RIGHTS_FIELD_FOLDERS -> a.folders.kind == b.folders.kind &&
        (a.folders.kind != FOLDERS_SELECTED ||
            (a.folders.paths.toSet() == b.folders.paths.toSet() && (a.folders.connections ?: false) == (b.folders.connections ?: false)))
    RIGHTS_FIELD_WRITE -> a.write == b.write
    RIGHTS_FIELD_VALID_UNTIL -> a.validUntil == b.validUntil
    RIGHTS_FIELD_ACCOUNT -> a.account == b.account
    else -> a.decision(field) == b.decision(field)
}

/**
 * What this phone can give: commands and terminal only while the exec provider runs, the screen
 * rows only when `execProvider.capabilities` names them (api.md §5). A right it cannot give is
 * shown off and sent off.
 */
internal data class OfferedRights(
    val commands: Boolean = true,
    val terminal: Boolean = true,
    val screenView: Boolean = true,
    val screenControl: Boolean = true,
    /** Why commands are not offered (the provider's detail), for the sheet. */
    val execNote: String? = null,
) {
    fun offers(field: String): Boolean = when (field) {
        RIGHTS_FIELD_COMMANDS -> commands
        RIGHTS_FIELD_TERMINAL -> terminal
        RIGHTS_FIELD_SCREEN_VIEW -> screenView
        RIGHTS_FIELD_SCREEN_CONTROL -> screenControl
        else -> true
    }

    /** [rights] with every right this phone cannot give turned off, and the account the app's own. */
    fun clamp(rights: Rights): Rights = rights.copy(
        commands = if (commands) rights.commands else ACCESS_OFF,
        terminal = if (terminal) rights.terminal else ACCESS_OFF,
        screenView = if (screenView) rights.screenView else ACCESS_OFF,
        screenControl = if (screenControl) rights.screenControl else ACCESS_OFF,
        account = RIGHTS_ACCOUNT_OWN,
    )

    companion object {
        fun from(provider: AccessProviderInfo?, exec: ExecProviderInfo?): OfferedRights {
            val keys = provider?.capabilities.orEmpty().map { it.key }.toSet()
            val running = exec?.available == true
            return OfferedRights(
                commands = running && (keys.isEmpty() || CAP_COMMANDS in keys),
                terminal = running && (keys.isEmpty() || CAP_TERMINAL in keys),
                screenView = CAP_SCREEN_VIEW in keys,
                screenControl = CAP_SCREEN_CONTROL in keys,
                execNote = if (running) null else exec?.detail?.takeIf { it.isNotBlank() } ?: "Befehle sind auf diesem Telefon nicht verfügbar.",
            )
        }
    }
}

internal const val EXPIRY_NONE = "none"
internal const val EXPIRY_KEEP = "keep"

/** The end choices of the sheet („Ablauf“): key, label and length in seconds (`null`: none or kept). */
internal data class ExpiryChoice(val key: String, val label: String, val seconds: Long?)

internal val EXPIRY_CHOICES = listOf(
    ExpiryChoice(EXPIRY_NONE, "Unbegrenzt", null),
    ExpiryChoice("1h", "1 Stunde", 3_600L),
    ExpiryChoice("1d", "1 Tag", 86_400L),
    ExpiryChoice("7d", "7 Tage", 7 * 86_400L),
)

/**
 * The state of the rights sheet. [known] are the fields whose current value this phone knows; an
 * unknown field is not shown as chosen and not sent unless it is touched or a preset fills it
 * („Rechte einstellen“ of an older connection does not guess its folders). [presetKey] is the last
 * preset chosen: the call then sends it plus only the fields that differ from it.
 */
internal data class RightsDraft(
    val rights: Rights = Rights(),
    val known: Set<String> = RIGHTS_FIELDS.toSet(),
    val presetKey: String? = null,
    val expiry: String = EXPIRY_NONE,
    val keptUntil: Long? = null,
) {
    fun knows(field: String): Boolean = field in known

    private fun touch(field: String): RightsDraft = copy(known = known + field)

    /** „Keine“, „Alle“ or „Auswahl“ of the folders; a selection keeps the paths already chosen. */
    fun setFolders(kind: String): RightsDraft {
        val selected = kind == FOLDERS_SELECTED
        val folders = RightsFolders(
            kind = kind,
            paths = if (selected) rights.folders.paths else emptyList(),
            connections = if (selected) rights.folders.connections ?: false else null,
        )
        return touch(RIGHTS_FIELD_FOLDERS).copy(rights = rights.copy(folders = folders))
    }

    /** Adds or removes one exact `path` of `exports` from the selection. */
    fun togglePath(path: String): RightsDraft {
        val paths = rights.folders.paths
        val next = if (path in paths) paths - path else paths + path
        return touch(RIGHTS_FIELD_FOLDERS).copy(
            rights = rights.copy(folders = RightsFolders(FOLDERS_SELECTED, next, rights.folders.connections ?: false)),
        )
    }

    fun setWrite(on: Boolean): RightsDraft = touch(RIGHTS_FIELD_WRITE).copy(rights = rights.copy(write = on))

    /**
     * Sets the decision of a capability. Controlling the screen needs watching (a control on raises
     * watching to it), and watching off ends controlling, like the access dialog.
     */
    fun setDecision(field: String, decision: String): RightsDraft {
        val next = when (field) {
            RIGHTS_FIELD_SCREEN_VIEW ->
                rights.copy(screenView = decision, screenControl = if (decision == ACCESS_OFF) ACCESS_OFF else rights.screenControl)
            RIGHTS_FIELD_SCREEN_CONTROL ->
                rights.copy(
                    screenControl = decision,
                    screenView = if (decision != ACCESS_OFF && rights.screenView == ACCESS_OFF) decision else rights.screenView,
                )
            else -> rights.withDecision(field, decision)
        }
        val touched = if (field == RIGHTS_FIELD_SCREEN_VIEW || field == RIGHTS_FIELD_SCREEN_CONTROL) {
            known + RIGHTS_FIELD_SCREEN_VIEW + RIGHTS_FIELD_SCREEN_CONTROL
        } else {
            known + field
        }
        return copy(rights = next, known = touched)
    }

    fun setExpiry(key: String): RightsDraft = touch(RIGHTS_FIELD_VALID_UNTIL).copy(expiry = key)

    /** Every field takes the preset's value (clamped to what this phone offers); the end stays as chosen. */
    fun applyPreset(info: RightsPresetInfo, offered: OfferedRights): RightsDraft =
        copy(rights = offered.clamp(info.rights), known = RIGHTS_FIELDS.toSet(), presetKey = info.key)

    /** The rights with the end resolved against [nowSeconds] and the app's own account. */
    fun effective(nowSeconds: Long): Rights {
        val until = when (expiry) {
            EXPIRY_NONE -> null
            EXPIRY_KEEP -> keptUntil
            else -> EXPIRY_CHOICES.firstOrNull { it.key == expiry }?.seconds?.let { nowSeconds + it }
        }
        return rights.copy(validUntil = until, account = RIGHTS_ACCOUNT_OWN, preset = null, summary = "")
    }

    /** The preset whose files and decisions the draft shows (its chip is selected), `null` for a custom state. */
    fun matchingPreset(presets: List<RightsPresetInfo>): RightsPresetInfo? {
        val presetFields = listOf(
            RIGHTS_FIELD_FOLDERS, RIGHTS_FIELD_WRITE, RIGHTS_FIELD_COMMANDS, RIGHTS_FIELD_TERMINAL,
            RIGHTS_FIELD_SCREEN_VIEW, RIGHTS_FIELD_SCREEN_CONTROL,
        )
        if (!presetFields.all { it in known }) return null
        return RightsPresets.matching(rights, presets)
    }

    /**
     * What the confirming call sends: the chosen preset plus the fields that differ from it, or
     * (no preset chosen) every known field. The account is always the app's own.
     */
    fun selection(presets: List<RightsPresetInfo>, nowSeconds: Long): RightsSelection {
        val value = effective(nowSeconds)
        val base = presetKey?.let { key -> presets.firstOrNull { it.key == key } }
        if (base != null) {
            val differing = RIGHTS_FIELDS.filter { it in known && !sameField(value, base.rights, it) }
            return RightsSelection(base.key, rightsFieldsJson(value, differing).takeIf { it.isNotEmpty() })
        }
        val sent = RIGHTS_FIELDS.filter { it in known || it == RIGHTS_FIELD_ACCOUNT }
        return RightsSelection(null, rightsFieldsJson(value, sent))
    }

    companion object {
        /** A draft that starts from [seed] (all fields known), shown clamped to what [offered] gives. */
        fun fromSeed(seed: Rights?, known: Set<String>, offered: OfferedRights): RightsDraft {
            val rights = offered.clamp(seed ?: Rights())
            val until = seed?.validUntil
            return RightsDraft(
                rights = rights,
                known = known,
                expiry = if (until != null) EXPIRY_KEEP else EXPIRY_NONE,
                keptUntil = until,
            )
        }
    }
}

/** „Dateien lesen und schreiben (alle Ordner) · Befehle · Terminal“; „Keine Rechte“ when nothing is given. */
internal fun rightsSummary(rights: Rights): String {
    val parts = buildList {
        val folders = rights.folders
        if (folders.kind != FOLDERS_NONE) {
            val scope = if (folders.kind == FOLDERS_ALL) "alle Ordner" else if (folders.paths.size == 1) "1 Ordner" else "${folders.paths.size} Ordner"
            add((if (rights.write) "Dateien lesen und schreiben" else "Dateien nur lesen") + " ($scope)")
        }
        decisionPart("Befehle", rights.commands)?.let(::add)
        decisionPart("Terminal", rights.terminal)?.let(::add)
        if (rights.screenControl != ACCESS_OFF) {
            add("Desktop sehen und steuern" + if (rights.screenControl == ACCESS_ASK || rights.screenView == ACCESS_ASK) " (fragt)" else "")
        } else if (rights.screenView != ACCESS_OFF) {
            add("Desktop sehen" + if (rights.screenView == ACCESS_ASK) " (fragt)" else "")
        }
    }
    val until = rights.validUntil?.let { " · bis ${Format.dateTime(it * 1000)}" }.orEmpty()
    return if (parts.isEmpty()) "Keine Rechte" else parts.joinToString(" · ") + until
}

private fun decisionPart(label: String, decision: String): String? = when (decision) {
    ACCESS_ALLOW -> label
    ACCESS_ASK -> "$label (fragt)"
    else -> null
}

/** German name of the way a connection came to wait for its rights (`PendingRights.origin`). */
internal fun originText(origin: String): String = when (origin) {
    "request" -> "Anfrage angenommen"
    "pairing" -> "per PIN gekoppelt"
    "share_back" -> "Rückfreigabe geöffnet"
    "repair" -> "Verbindung repariert"
    "identity_replaced" -> "Gerät mit neuem Schlüssel"
    "readmitted" -> "wieder zugelassen"
    "room_created" -> "Raum erstellt"
    "room_joined" -> "Raum beigetreten"
    else -> "neue Verbindung"
}

/** Everything the rights sheet needs about the connection it opens for. */
internal data class RightsRequest(
    val target: RightsTarget,
    val name: String,
    val origin: String?,
    val seed: Rights?,
    val known: Set<String>,
    /** The connection waits for this confirmation: „Abbrechen“ then calls `share.cancelRights`. */
    val pending: Boolean,
) {
    val key: String
        get() = target.key

    /** The scope of the folder list: `direct` or the room's profile id. */
    val folderScope: String
        get() = if (target.isRoom) target.profileId.orEmpty() else SHARE_DIRECT
}

internal fun requestOfPending(pending: PendingRights): RightsRequest = RightsRequest(
    target = pending.target,
    name = pending.name,
    origin = pending.origin,
    seed = pending.rights,
    known = RIGHTS_FIELDS.toSet(),
    pending = true,
)

/** The pins of a Direct device row; `null` while the device has no confirmed identity. */
internal fun rightsTargetOf(device: ShareDevice): RightsTarget? {
    val id = device.deviceId?.takeIf { it.isNotBlank() } ?: return null
    val key = device.publicKey?.takeIf { it.isNotBlank() } ?: return null
    return RightsTarget.direct(id, key)
}

internal fun pendingFor(target: RightsTarget?, pending: List<PendingRights>): PendingRights? =
    target?.let { wanted -> pending.firstOrNull { it.target.key == wanted.key } }

/** The first open confirmation the user did not put off in this session. */
internal fun nextPendingSheet(pending: List<PendingRights>, postponed: Set<String>): PendingRights? =
    pending.firstOrNull { it.target.key !in postponed }

/** The sheet to show: an explicit one (menu) wins, else the next open confirmation. */
internal fun resolveRightsSheet(explicit: RightsRequest?, pending: List<PendingRights>, postponed: Set<String>): RightsRequest? =
    explicit ?: nextPendingSheet(pending, postponed)?.let(::requestOfPending)

/**
 * „Rechte einstellen“ for a connection that is not waiting: starts from what the status shows (the
 * write right, the commands, terminal and screen decisions, the end) and leaves the folders open
 * (their selection is not part of the status). An open confirmation of the target opens as such.
 */
internal fun requestOfExisting(target: RightsTarget, name: String, status: ShareStatus?, access: AccessOverview?): RightsRequest {
    pendingFor(target, status?.pendingRights.orEmpty())?.let { return requestOfPending(it) }
    val known = linkedSetOf(RIGHTS_FIELD_ACCOUNT)
    var rights = Rights()
    if (target.isRoom) {
        status?.rooms?.firstOrNull { it.profileId == target.profileId }?.policy?.let {
            rights = rights.copy(write = it.membersMayWrite)
            known += RIGHTS_FIELD_WRITE
        }
    } else {
        val grant = status?.writeGrants?.firstOrNull { it.deviceId == target.deviceId && it.publicKey == target.publicKey }
        val device = status?.devices?.firstOrNull { it.deviceId == target.deviceId && it.publicKey == target.publicKey }
        (grant?.takeIf { it.active }?.write ?: device?.write)?.let {
            rights = rights.copy(write = it)
            known += RIGHTS_FIELD_WRITE
        }
        val exec = status?.execTargets?.firstOrNull { it.relation == "direct" && it.deviceId == target.deviceId }
        val view = exec?.let { access?.accessOf(it.targetKey) }
        if (view != null) {
            rights = rights.copy(
                commands = view.decision(CAP_COMMANDS),
                terminal = view.decision(CAP_TERMINAL),
                screenView = view.decision(CAP_SCREEN_VIEW),
                screenControl = view.decision(CAP_SCREEN_CONTROL),
                validUntil = view.validUntil?.takeIf { !view.expired },
            )
            known += listOf(RIGHTS_FIELD_COMMANDS, RIGHTS_FIELD_TERMINAL, RIGHTS_FIELD_SCREEN_VIEW, RIGHTS_FIELD_SCREEN_CONTROL, RIGHTS_FIELD_VALID_UNTIL)
        }
    }
    return RightsRequest(target, name, origin = null, seed = rights, known = known, pending = false)
}

/** The folders the sheet offers for [request]: the Direct folders, or those of the room. */
internal fun folderChoices(status: ShareStatus?, request: RightsRequest): List<ShareExport> =
    if (request.target.isRoom) status?.exports?.rooms?.get(request.folderScope).orEmpty() else status?.exports?.direct.orEmpty()

/** Name of the room of [request]'s target for titles; the status knows it by profile id. */
internal fun roomOf(status: ShareStatus?, target: RightsTarget): ShareRoom? =
    status?.rooms?.firstOrNull { it.profileId == target.profileId }

/** „Rechte für Laptop“. */
internal fun rightsTitle(request: RightsRequest): String = "Rechte für ${request.name.ifBlank { if (request.target.isRoom) "Raum" else "Gerät" }}"

/** One line under the title: how the connection came about and what it can do until the user confirms. */
internal fun rightsIntro(request: RightsRequest): String = when {
    request.pending && request.target.isRoom ->
        "${request.origin?.let(::originText)?.replaceFirstChar { it.uppercase() } ?: "Neuer Raum"}. Bis zur Bestätigung gibt der Raum nichts frei."
    request.pending ->
        "${request.origin?.let(::originText)?.replaceFirstChar { it.uppercase() } ?: "Neue Verbindung"}. Bis zur Bestätigung hat das Gerät keine Rechte."
    else -> "Gilt für genau diese Verbindung. Nicht angezeigte Werte bleiben, wie sie sind."
}

/** Text of the confirming button: names what is given. */
internal fun confirmButtonLabel(rights: Rights): String = if (rights.folders.kind == FOLDERS_NONE &&
    rights.commands == ACCESS_OFF && rights.terminal == ACCESS_OFF && rights.screenView == ACCESS_OFF && rights.screenControl == ACCESS_OFF
) "Ohne Rechte bestätigen" else "Bestätigen"

/**
 * What a call sends for the preset [key] chosen in advance (the Direct code, a PIN offer, a new room,
 * accepting a request): the preset plus the fields this phone has to turn off; `null` for no choice.
 */
internal fun presetSelection(key: String?, presets: List<RightsPresetInfo>, offered: OfferedRights, nowSeconds: Long = 0L): RightsSelection? {
    val info = presets.firstOrNull { it.key == key } ?: return null
    return RightsDraft().applyPreset(info, offered).selection(presets, nowSeconds)
}

/** What accepting a request gives: the chosen preset, else the Direct code's rights, else nothing yet. */
internal fun acceptRightsText(chosen: RightsPresetInfo?, codeRights: Rights?): String = when {
    chosen != null && !grantsAnything(chosen.rights) ->
        "Das Gerät wird ohne Rechte verbunden. Du kannst sie später unter „Rechte einstellen“ geben."
    chosen != null -> "Das Gerät erhält sofort: ${rightsSummary(chosen.rights)}."
    codeRights != null && !grantsAnything(codeRights) ->
        "Dein Direct-Code vergibt keine Rechte: das Gerät wird ohne Rechte verbunden."
    codeRights != null -> "Das Gerät erhält die Rechte deines Direct-Codes: ${rightsSummary(codeRights)}."
    else -> "Das Gerät wird verbunden, hat aber noch keine Rechte. Danach öffnet sich „Rechte für …“, in dem du sie festlegst."
}

/** Whether [rights] allow anything at all (a folder, writing, a command, the terminal or the screen). */
internal fun grantsAnything(rights: Rights): Boolean = rights.folders.kind != FOLDERS_NONE || rights.write ||
    listOf(rights.commands, rights.terminal, rights.screenView, rights.screenControl).any { it != ACCESS_OFF }

/**
 * What new connections get (Direct code, PIN pairing, "open my shares" and repair; also automatically
 * accepted devices), for its line.
 */
internal fun codeRightsText(codeRights: Rights?): String = when {
    codeRights == null -> "Wird beim Verbinden erfragt: das Gerät wartet ohne Rechte („Rechte offen“), bis du sie bestätigst."
    !grantsAnything(codeRights) -> "Neue Verbindungen (Direkt-Code, PIN-Kopplung) erhalten keine Rechte: Geräte sind verbunden, mehr nicht."
    else -> "Neue Verbindungen (Direkt-Code, PIN-Kopplung) erhalten sofort: ${rightsSummary(codeRights)}."
}

/**
 * What a device that pairs with the running PIN offer gets (the offer's `rights`, plan 2026-10-31
 * F5): the rights the worker holds, or that it waits for the confirmation. A room's offer carries
 * none; the room's own rights apply.
 */
internal fun offerRightsText(rights: Rights?, room: Boolean = false): String = when {
    room -> "Die Rechte der Mitglieder legt der Raum fest."
    rights == null -> "Das Gerät, das mit dieser PIN verbindet, wartet ohne Rechte; du legst sie danach fest."
    !grantsAnything(rights) -> "Das Gerät, das mit dieser PIN verbindet, wird ohne Rechte verbunden."
    else -> "Das Gerät, das mit dieser PIN verbindet, erhält: ${rightsSummary(rights)}."
}

/** The sentence of a request policy hint: whether new devices are accepted automatically and with which rights. */
internal fun requestPolicyHint(requests: String?, codeRights: Rights?): String = when (requests) {
    "AutoAccept" ->
        "Neue Zugriffsanfragen werden automatisch angenommen. " + when {
            codeRights == null -> "Das Gerät wartet danach ohne Rechte auf deine Bestätigung („Rechte offen“)."
            !grantsAnything(codeRights) -> "Das Gerät ist danach verbunden, hat aber keine Rechte."
            else -> "Es erhält sofort: ${rightsSummary(codeRights)}."
        }
    "Ask" -> "Neue Zugriffsanfragen warten auf deine Zustimmung."
    else -> "Anfrageeinstellung noch nicht geladen."
}

/** What a button of the rights sheet asks the bridge to do. */
internal sealed interface RightsCall {
    /** „Bestätigen“: `share.confirmRights` for [target] with [selection]. */
    data class Confirm(val target: RightsTarget, val selection: RightsSelection) : RightsCall

    /** „Abbrechen“ of a waiting connection: `share.cancelRights`; it stays connected, without rights. */
    data class Cancel(val target: RightsTarget) : RightsCall

    /** „Abbrechen“ of a menu for a connection that does not wait: nothing is called, nothing changes. */
    data object CloseOnly : RightsCall
}

internal fun confirmCall(request: RightsRequest, draft: RightsDraft, presets: List<RightsPresetInfo>, nowSeconds: Long): RightsCall.Confirm =
    RightsCall.Confirm(request.target, draft.selection(presets, nowSeconds))

internal fun cancelCall(request: RightsRequest): RightsCall = if (request.pending) RightsCall.Cancel(request.target) else RightsCall.CloseOnly
