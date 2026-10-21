package app.smartexplorer.android.launcher.se

import app.smartexplorer.android.launcher.bridge.DeviceEntry
import app.smartexplorer.android.launcher.bridge.DevicesData
import app.smartexplorer.android.launcher.bridge.MemberEntry
import app.smartexplorer.android.launcher.bridge.RoomEntry
import app.smartexplorer.android.launcher.bridge.StateTexts
import app.smartexplorer.android.launcher.bridge.StatusCodes
import app.smartexplorer.android.launcher.bridge.Tone
import app.smartexplorer.android.launcher.bridge.VaultEntry
import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.model.Kinds

/** What the launcher knows about the se elements: the last `DEVICES` (live or from the cache) and the bridge state. */
data class SeUiState(val data: DevicesData? = null, val bridge: BridgeState = BridgeState.Idle) {
    /** Current data: they came from the service in this run and the bridge still stands (U40-9, „Entfernt“). */
    val fresh: Boolean get() = data != null && !data.stale && bridge is BridgeState.Connected

    /** The group „Smart Explorer antwortet nicht“ with „Erneut“ (C4.6). */
    val unreachable: Boolean get() = bridge is BridgeState.Unreachable
}

/** How one tile looks (U40-9). [gone] is „Nicht mehr vorhanden“: the target left the current data. */
data class TileView(
    val title: String,
    val subtitle: String?,
    val tone: Tone,
    val progress: Boolean,
    val gone: Boolean,
    val description: String,
    val code: String? = null,
)

/** The pure part of the tiles: from an item and the state to its text, tone and description. */
object SeViews {
    const val ALL_SCREENS = "Alle Bildschirme"
    private const val DEVICE_FALLBACK = "Gerät"
    private const val ROOM_FALLBACK = "Raum"
    private const val VAULT_FALLBACK = "Tresor"

    fun device(item: HomeItem, ui: SeUiState): TileView {
        val target = item.attr("target").orEmpty()
        val data = ui.data
        val found = data?.let { findDevice(it, target) }
        if (found != null) return deviceView(found.name, found.status, found.statusText, found.lan, ui)
        val name = item.attr("name") ?: DEVICE_FALLBACK
        return if (ui.fresh) goneView(name) else unknownDevice(name, ui)
    }

    /** A member in the popup of a room. */
    fun member(member: MemberEntry, ui: SeUiState): TileView = deviceView(member.name, member.status, member.statusText, false, ui)

    private fun deviceView(name: String, code: String, statusText: String, lan: Boolean, ui: SeUiState): TileView {
        if (ui.bridge is BridgeState.Binding && !ui.fresh) {
            return TileView(name, StateTexts.text(StatusCodes.CONNECTING), Tone.Progress, true, false, "$name, ${StateTexts.text(StatusCodes.CONNECTING)}")
        }
        val stale = ui.data?.stale == true
        val state = StateTexts.withStale(StateTexts.text(code, lan), stale)
        val detail = if (code == StatusCodes.FAILED) ", ${StatusCodes.errorDetail(statusText)}" else ""
        return TileView(name, state, StateTexts.tone(code), code == StatusCodes.CONNECTING, false, "$name, $state$detail", code)
    }

    private fun unknownDevice(name: String, ui: SeUiState): TileView {
        val state = if (ui.bridge is BridgeState.Binding) StateTexts.text(StatusCodes.CONNECTING) else StateTexts.text(StatusCodes.UNKNOWN)
        val progress = ui.bridge is BridgeState.Binding
        return TileView(name, state, if (progress) Tone.Progress else Tone.Neutral, progress, false, "$name, $state", StatusCodes.UNKNOWN)
    }

    private fun goneView(name: String) = TileView(name, StateTexts.GONE, Tone.Gray, false, true, "$name, ${StateTexts.GONE}")

    fun room(item: HomeItem, ui: SeUiState): TileView {
        val room = ui.data?.rooms?.firstOrNull { it.profileId == item.attr("target") }
        if (room == null) {
            val name = item.attr("name") ?: ROOM_FALLBACK
            return if (ui.fresh) goneView(name) else TileView(name, null, Tone.Neutral, false, false, name)
        }
        val count = membersText(room)
        return TileView(room.name, StateTexts.withStale(count, ui.data?.stale == true), Tone.Neutral, false, false, "${room.name}, $count")
    }

    fun membersText(room: RoomEntry): String = if (room.members.size == 1) "1 Gerät" else "${room.members.size} Geräte"

    fun vault(item: HomeItem, ui: SeUiState): TileView {
        val site = item.attr("target").orEmpty().removePrefix("vault://").trimEnd('/')
        val entry = ui.data?.vaults?.firstOrNull { it.site == site }
        if (entry == null) {
            val name = item.attr("name") ?: VAULT_FALLBACK
            return if (ui.fresh) goneView(name) else TileView(name, null, Tone.Neutral, false, false, name)
        }
        val state = StateTexts.withStale(StateTexts.vault(entry.state), ui.data?.stale == true)
        return TileView(entry.label.ifEmpty { VAULT_FALLBACK }, state, Tone.Neutral, false, false, "${entry.label}, $state")
    }

    fun screens(): TileView = TileView(ALL_SCREENS, null, Tone.Neutral, false, false, ALL_SCREENS)

    /** The title the layout stores as `attrs.name` and the label of the accessibility service. */
    fun title(item: HomeItem, ui: SeUiState): String = when (item.kind) {
        Kinds.SE_DEVICE -> device(item, ui).title
        Kinds.SE_ROOM -> room(item, ui).title
        Kinds.SE_VAULT -> vault(item, ui).title
        Kinds.SE_SCREENS -> ALL_SCREENS
        Kinds.SE_ADD_DEVICE -> "Gerät hinzufügen"
        Kinds.SE_JOIN_ROOM -> "Raum beitreten"
        else -> ""
    }

    /** A device of the list or a member of a room by its `location`. */
    fun findDevice(data: DevicesData, location: String): DeviceEntry? {
        val wanted = location.trimEnd('/')
        return data.devices.firstOrNull { it.location.trimEnd('/') == wanted } ?: data.rooms.firstNotNullOfOrNull { room ->
            room.members.firstOrNull { it.location.trimEnd('/') == wanted }?.let { m ->
                DeviceEntry(m.deviceId, m.name, m.status, m.statusText, m.online, m.location, false, false, null, true, false, m.key, m.screen)
            }
        }
    }

    fun findVault(data: DevicesData, site: String): VaultEntry? = data.vaults.firstOrNull { it.site == site }
}
