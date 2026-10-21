package app.smartexplorer.android.service

import android.content.Context
import android.util.Log
import app.smartexplorer.android.api.DocsApi
import app.smartexplorer.android.api.DocsSettings
import app.smartexplorer.android.api.ScreenApi
import app.smartexplorer.android.api.ScreenDevice
import app.smartexplorer.android.api.ShareApi
import app.smartexplorer.android.api.ShareStatus
import app.smartexplorer.android.api.VaultApi
import app.smartexplorer.android.api.VaultList
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.launcher.bridge.DevicesData
import app.smartexplorer.android.launcher.bridge.DevicesFolder
import app.smartexplorer.android.launcher.bridge.DevicesJson
import app.smartexplorer.android.launcher.bridge.DeviceEntry
import app.smartexplorer.android.launcher.bridge.MemberEntry
import app.smartexplorer.android.launcher.bridge.RoomEntry
import app.smartexplorer.android.launcher.bridge.ScreenRef
import app.smartexplorer.android.launcher.bridge.StatusCodes
import app.smartexplorer.android.launcher.bridge.VaultEntry
import app.smartexplorer.android.launcher.se.DeviceKey
import app.smartexplorer.android.system.documents.DocumentIds

/**
 * What the main process tells the launcher about devices, rooms, vaults and the screens (DSK-M13, U40-5): one reading
 * of `share.status`, `vault.list`, `screen.devices` and `docs.settings.get`. The bridge service and the widget feed
 * use only [build] and [toJson]; the process `:launcher` never calls the core.
 *
 * Only the fields the launcher needs are kept: no keys, node ids, fingerprints, admission, policy or auto-join of the
 * rooms, and of a vault only site, label and state — never a name, size or picture of its content (S5-V5, S3).
 */
class LauncherSnapshot internal constructor(internal val data: DevicesData) {
    /** `share.status` failed: nothing else is valid, the launcher shows its error group (C4.6). */
    val error: String? get() = data.error

    companion object {
        private const val TAG = "LauncherSnapshot"

        /**
         * Reads the four sources. Each can fail alone (`CoreException`): `vaults` is then empty, `screen` is `null`,
         * `devicesFolder` is `null`; only a failing `share.status` makes the whole snapshot an error.
         */
        suspend fun build(context: Context): LauncherSnapshot {
            val status = try {
                ShareApi.status()
            } catch (e: CoreException) {
                Log.w(TAG, "share.status failed: ${e.kind}: ${e.message}")
                return LauncherSnapshot(DevicesData(error = e.message ?: e.kind))
            }
            val vaults = optional { VaultApi.list() }
            val screens = optional { ScreenApi.devices() }
            val docs = optional { DocsApi.settings() }
            return LauncherSnapshot(assemble(status, vaults, screens, docs, DocumentIds.authority(context)))
        }

        /** The message text for [snapshot] with the token of this installation (U40-12). */
        fun toJson(snapshot: LauncherSnapshot, navToken: String): String =
            DevicesJson.encode(snapshot.data.copy(navToken = navToken, stale = false), withToken = true)

        private suspend fun <T> optional(read: suspend () -> T): T? = try {
            read()
        } catch (e: CoreException) {
            Log.w(TAG, "source unavailable: ${e.kind}: ${e.message}")
            null
        }

        /** The pure part of [build], reachable from JVM tests. */
        internal fun assemble(
            status: ShareStatus,
            vaults: VaultList?,
            screens: List<ScreenDevice>?,
            docs: DocsSettings?,
            authority: String,
        ): DevicesData {
            val screenRows = screens.orEmpty()
            fun screenOf(relation: String, relationId: String, deviceId: String?): ScreenRef? =
                screenRows.firstOrNull {
                    it.relation == relation && it.relationId == relationId && (deviceId == null || it.deviceId == deviceId)
                }?.let { ScreenRef(it.relation, it.relationId, it.deviceId) }

            val devices = status.devices.map { d ->
                DeviceEntry(
                    contactId = d.contactId,
                    name = d.name,
                    status = d.status,
                    statusText = d.statusText,
                    online = d.online,
                    location = d.location,
                    lan = d.lan,
                    shareBack = d.shareBack,
                    write = d.write,
                    autoConnect = d.autoConnect,
                    autoOpen = d.autoOpen,
                    key = DeviceKey.of(d.location),
                    screen = screenOf("direct", d.contactId, null),
                )
            }
            val rooms = status.rooms.map { room ->
                RoomEntry(
                    profileId = room.profileId,
                    roomId = room.roomId,
                    name = room.name,
                    status = room.status,
                    members = room.members.map { m ->
                        val code = StatusCodes.fromLabel(m.status)
                        MemberEntry(
                            deviceId = m.deviceId,
                            name = m.name,
                            status = code,
                            statusText = StatusCodes.statusTextOf(m.status),
                            online = StatusCodes.isOnline(code),
                            location = m.location,
                            blocked = m.blocked,
                            key = DeviceKey.of(m.location),
                            screen = screenOf("room", room.profileId, m.deviceId),
                        )
                    },
                )
            }
            val sites = vaults?.vaults.orEmpty().flatMap { it.sites }.map { VaultEntry(it.site, it.label, it.state) }
            val folder = docs?.takeIf { it.provider && it.devices }?.let { DevicesFolder(authority, DocumentIds.DEVICES) }
            return DevicesData(devices = devices, rooms = rooms, vaults = sites, devicesFolder = folder)
        }
    }
}
