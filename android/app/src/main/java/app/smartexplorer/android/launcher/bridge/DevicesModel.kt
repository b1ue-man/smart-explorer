package app.smartexplorer.android.launcher.bridge

/** The screen of a device in the form of `screen.devices` (U40-5): [relation] `direct` or `room`. */
data class ScreenRef(val relation: String, val relationId: String, val deviceId: String)

/** A direct device: the eleven keys of `share.status` plus [key] and [screen] (13 in all, U40-4). */
data class DeviceEntry(
    val contactId: String,
    val name: String,
    val status: String,
    val statusText: String,
    val online: Boolean,
    val location: String,
    val lan: Boolean,
    val shareBack: Boolean,
    val write: Boolean?,
    val autoConnect: Boolean,
    val autoOpen: Boolean,
    val key: String?,
    val screen: ScreenRef?,
)

/** A member of a room; [status] is the status code, [statusText] the label (U40-5). */
data class MemberEntry(
    val deviceId: String,
    val name: String,
    val status: String,
    val statusText: String,
    val online: Boolean,
    val location: String,
    val blocked: Boolean,
    val key: String?,
    val screen: ScreenRef?,
)

data class RoomEntry(val profileId: String, val roomId: String, val name: String, val status: String, val members: List<MemberEntry>)

/** An entered vault site: [state] `locked`, `unlocked` or `unavailable`. Nothing of its content (S5-V5). */
data class VaultEntry(val site: String, val label: String, val state: String)

/** The root of the documents provider that shows the devices (C4.8). */
data class DevicesFolder(val authority: String, val rootId: String)

/**
 * What the main process tells the launcher (`DEVICES`, U40-4). [error] set means nothing else is valid. [navToken]
 * is `null` in the cache file. [stale] is `false` in the message of the service and `true` in the cache.
 */
data class DevicesData(
    val navToken: String? = null,
    val stale: Boolean = false,
    val devices: List<DeviceEntry> = emptyList(),
    val rooms: List<RoomEntry> = emptyList(),
    val vaults: List<VaultEntry> = emptyList(),
    val devicesFolder: DevicesFolder? = null,
    val error: String? = null,
) {
    /** True when there is neither a direct device nor a room (the groups „Gerät hinzufügen“ and „Raum beitreten“, U40-9). */
    val noDevicesOrRooms: Boolean get() = devices.isEmpty() && rooms.isEmpty()
}
