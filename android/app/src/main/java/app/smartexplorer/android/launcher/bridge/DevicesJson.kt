package app.smartexplorer.android.launcher.bridge

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.put

/**
 * The JSON of `DEVICES` and of the cache file (U40-4, U40-8):
 * `{"v":1,"navToken":…,"stale":false,"devices":[…],"rooms":[…],"vaults":[…],"devicesFolder":{…}|null}` or
 * `{"v":1,"error":"…"}`. Pure Kotlin; the service writes with [encode], the launcher reads with [parse].
 */
object DevicesJson {
    private val json = Json { ignoreUnknownKeys = true }

    fun encodeError(text: String): String = buildJsonObject {
        put("v", BridgeProtocol.VERSION)
        put("error", text)
    }.toString()

    /** [withToken] false writes the cache form: no `navToken`, and `stale` true. */
    fun encode(data: DevicesData, withToken: Boolean): String {
        data.error?.let { return encodeError(it) }
        return buildJsonObject {
            put("v", BridgeProtocol.VERSION)
            if (withToken) put("navToken", data.navToken)
            put("stale", if (withToken) data.stale else true)
            put("devices", buildJsonArray { data.devices.forEach { add(device(it)) } })
            put("rooms", buildJsonArray { data.rooms.forEach { add(room(it)) } })
            put(
                "vaults",
                buildJsonArray {
                    data.vaults.forEach { add(buildJsonObject { put("site", it.site); put("label", it.label); put("state", it.state) }) }
                },
            )
            put(
                "devicesFolder",
                data.devicesFolder?.let { buildJsonObject { put("authority", it.authority); put("rootId", it.rootId) } } ?: JsonNull,
            )
        }.toString()
    }

    private fun screen(ref: ScreenRef?): JsonElement = ref?.let {
        buildJsonObject {
            put("relation", it.relation)
            put("relationId", it.relationId)
            put("deviceId", it.deviceId)
        }
    } ?: JsonNull

    private fun device(d: DeviceEntry) = buildJsonObject {
        put("contactId", d.contactId)
        put("name", d.name)
        put("status", d.status)
        put("statusText", d.statusText)
        put("online", d.online)
        put("location", d.location)
        put("lan", d.lan)
        put("shareBack", d.shareBack)
        put("write", d.write)
        put("autoConnect", d.autoConnect)
        put("autoOpen", d.autoOpen)
        put("key", d.key)
        put("screen", screen(d.screen))
    }

    private fun room(r: RoomEntry) = buildJsonObject {
        put("profileId", r.profileId)
        put("roomId", r.roomId)
        put("name", r.name)
        put("status", r.status)
        put(
            "members",
            buildJsonArray {
                r.members.forEach { m ->
                    add(
                        buildJsonObject {
                            put("deviceId", m.deviceId)
                            put("name", m.name)
                            put("status", m.status)
                            put("statusText", m.statusText)
                            put("online", m.online)
                            put("location", m.location)
                            put("blocked", m.blocked)
                            put("key", m.key)
                            put("screen", screen(m.screen))
                        },
                    )
                }
            },
        )
    }

    /** `null` for text that is no object of version 1; tolerant of missing fields, since the cache file may be older. */
    fun parse(text: String): DevicesData? {
        val root = try {
            json.parseToJsonElement(text) as? JsonObject
        } catch (_: IllegalArgumentException) {
            null
        } ?: return null
        if (root.int("v") != BridgeProtocol.VERSION) return null
        root.str("error")?.let { return DevicesData(error = it) }
        return DevicesData(
            navToken = root.str("navToken"),
            stale = root.bool("stale") ?: false,
            devices = root.objects("devices").mapNotNull { parseDevice(it) },
            rooms = root.objects("rooms").mapNotNull { parseRoom(it) },
            vaults = root.objects("vaults").mapNotNull { v ->
                v.str("site")?.let { VaultEntry(it, v.str("label").orEmpty(), v.str("state") ?: "locked") }
            },
            devicesFolder = (root["devicesFolder"] as? JsonObject)?.let { f ->
                val authority = f.str("authority")
                val rootId = f.str("rootId")
                if (authority != null && rootId != null) DevicesFolder(authority, rootId) else null
            },
        )
    }

    private fun parseScreen(o: JsonObject?): ScreenRef? {
        o ?: return null
        val relation = o.str("relation") ?: return null
        val relationId = o.str("relationId") ?: return null
        val deviceId = o.str("deviceId") ?: return null
        return ScreenRef(relation, relationId, deviceId)
    }

    private fun parseDevice(o: JsonObject): DeviceEntry? {
        val contactId = o.str("contactId") ?: return null
        val location = o.str("location") ?: return null
        return DeviceEntry(
            contactId = contactId,
            name = o.str("name").orEmpty(),
            status = o.str("status") ?: StatusCodes.UNKNOWN,
            statusText = o.str("statusText").orEmpty(),
            online = o.bool("online") ?: false,
            location = location,
            lan = o.bool("lan") ?: false,
            shareBack = o.bool("shareBack") ?: false,
            write = o.bool("write"),
            autoConnect = o.bool("autoConnect") ?: true,
            autoOpen = o.bool("autoOpen") ?: false,
            key = o.str("key"),
            screen = parseScreen(o["screen"] as? JsonObject),
        )
    }

    private fun parseRoom(o: JsonObject): RoomEntry? {
        val profileId = o.str("profileId") ?: return null
        return RoomEntry(
            profileId = profileId,
            roomId = o.str("roomId").orEmpty(),
            name = o.str("name").orEmpty(),
            status = o.str("status").orEmpty(),
            members = o.objects("members").mapNotNull { m ->
                val deviceId = m.str("deviceId") ?: return@mapNotNull null
                val location = m.str("location") ?: return@mapNotNull null
                MemberEntry(
                    deviceId = deviceId,
                    name = m.str("name").orEmpty(),
                    status = m.str("status") ?: StatusCodes.UNKNOWN,
                    statusText = m.str("statusText").orEmpty(),
                    online = m.bool("online") ?: false,
                    location = location,
                    blocked = m.bool("blocked") ?: false,
                    key = m.str("key"),
                    screen = parseScreen(m["screen"] as? JsonObject),
                )
            },
        )
    }

    private fun JsonObject.str(name: String): String? = (this[name] as? JsonPrimitive)?.takeIf { it !is JsonNull }?.contentOrNull

    private fun JsonObject.bool(name: String): Boolean? = (this[name] as? JsonPrimitive)?.booleanOrNull

    private fun JsonObject.int(name: String): Int? = (this[name] as? JsonPrimitive)?.intOrNull

    private fun JsonObject.objects(name: String): List<JsonObject> = (this[name] as? JsonArray).orEmpty().mapNotNull { it as? JsonObject }
}
