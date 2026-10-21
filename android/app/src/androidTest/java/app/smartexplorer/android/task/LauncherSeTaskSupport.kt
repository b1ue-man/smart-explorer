package app.smartexplorer.android.task

import app.smartexplorer.android.launcher.bridge.BridgeProtocol
import app.smartexplorer.android.launcher.bridge.DevicesData
import app.smartexplorer.android.launcher.bridge.DevicesJson
import app.smartexplorer.android.launcher.bridge.DeviceEntry
import app.smartexplorer.android.launcher.bridge.MemberEntry
import app.smartexplorer.android.launcher.bridge.RoomEntry
import app.smartexplorer.android.launcher.bridge.VaultEntry
import java.io.File
import kotlinx.serialization.json.JsonObject

/**
 * Helpers of the DSK-M13 tests in `LauncherTaskTest` (U40-21): layouts with se tiles, a cache file for the launcher, and the
 * pairing with the desktop of the host script. Kept apart so the test class stays readable.
 */
internal object LauncherSeTaskSupport {
    /** One item of the layout file: [kind] at cell ([x], [y]) with `target` and `name`, the form of `LayoutJson` (U35-18). */
    fun item(id: String, kind: String, x: Int, y: Int, target: String? = null, name: String? = null): String = buildString {
        append("""{"id":"$id","kind":"$kind","cell":[$x,$y],"span":[1,1]""")
        target?.let { append(""","target":"$it"""") }
        name?.let { append(""","name":"$it"""") }
        append("}")
    }

    /** A layout of one page with [items] and the first fill already done, so the launcher adds nothing of its own. */
    fun layout(vararg items: String): String =
        """{"schema":1,"grid":{"columns":4,"rows":5,"iconDp":56},"pages":[{"items":[${items.joinToString(",")}]}],"dock":[],"seSeeded":true}"""

    /** The cache file of the launcher for [data] (U40-8): stale and without the token. */
    fun writeCache(launcherDir: File, data: DevicesData) {
        launcherDir.mkdirs()
        File(launcherDir, BridgeProtocol.SE_CACHE_FILE).writeText(DevicesJson.encode(data, withToken = false))
    }

    fun deviceEntry(id: String, name: String, status: String = "offline", statusText: String = "Offline", online: Boolean = false) = DeviceEntry(
        contactId = id, name = name, status = status, statusText = statusText, online = online, location = "share://direct/$id", lan = false,
        shareBack = false, write = null, autoConnect = true, autoOpen = false, key = "direct~$id", screen = null,
    )

    fun roomEntry(profileId: String, name: String, vararg members: Pair<String, String>) = RoomEntry(
        profileId, profileId, name, "Verbunden",
        members.map { (id, memberName) ->
            MemberEntry(id, memberName, "available", "Online", true, "share://room/$profileId/$id", false, "room~$profileId~$id", null)
        },
    )

    fun vaultEntry(site: String, label: String, state: String = "locked") = VaultEntry(site, label, state)

    /** The Share pair of the host script (`seShareServer`, `seDesktopDevice`, `seDesktopDirectCode`): the phone's direct contact of the runner. */
    suspend fun pairWithRunner(): JsonObject {
        Share.goOnline(TaskArgs.get("seShareServer"))
        val existing = Api.obj("share.status").objects("devices").firstOrNull()
        if (existing != null) return existing
        val contactId = Api.obj("share.addDirect", args("code" to TaskArgs.get("seDesktopDirectCode"), "name" to "Runner")).text("contactId")
        var device: JsonObject? = null
        waitFor("Kontakt $contactId in share.status", 60_000) {
            device = Api.obj("share.status").objects("devices").firstOrNull { it.text("contactId") == contactId }
            device != null
        }
        return device!!
    }

    /** True when the host script gave the arguments of the Share pair; the tests that need it are „nicht ausgeführt“ otherwise. */
    fun hasPairArguments(): Boolean = listOf("seShareServer", "seDesktopDevice", "seDesktopDirectCode").all {
        !androidx.test.platform.app.InstrumentationRegistry.getArguments().getString(it).isNullOrBlank()
    }
}
