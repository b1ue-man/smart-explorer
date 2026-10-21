package app.smartexplorer.android.launcher.se

import app.smartexplorer.android.launcher.bridge.DevicesData
import app.smartexplorer.android.launcher.bridge.StateTexts
import app.smartexplorer.android.launcher.search.LauncherSearch
import app.smartexplorer.android.launcher.search.SearchHit
import app.smartexplorer.android.launcher.search.SearchSource

/**
 * The section „Smart Explorer“ of the app search (U40-15): the names of all devices, room members, rooms, vault labels
 * and the fixed text „Alle Bildschirme“ from the last `DEVICES`; the cache is enough. A hit opens like the tile. The
 * search itself never binds the service (rule C3.3); only opening a hit is the person's act.
 */
object SeSearchSource : SearchSource {
    const val SECTION = "Smart Explorer"

    /** Set by the activity before the source is registered; `null` finds nothing. */
    @Volatile
    var controller: SeController? = null

    override fun search(tokens: List<String>): List<SearchHit> {
        val se = controller ?: return emptyList()
        val ui = se.ui.value
        val data = ui.data ?: return emptyList()
        val stale = data.stale
        return hits(data, tokens).sortedWith(compareBy({ it.rank }, { it.title.lowercase() })).map { hit ->
            SearchHit(SECTION, hit.title, hit.subtitle?.let { StateTexts.withStale(it, stale) }) {
                se.open(se.appContext, hit.target)
            }
        }
    }

    internal class Hit(val title: String, val subtitle: String?, val target: NavTarget, val rank: Int)

    /** The matches of [tokens] in [data], with the rank of the app search (0: every word starts a word, 1: occurs). */
    internal fun hits(data: DevicesData, tokens: List<String>): List<Hit> {
        val out = ArrayList<Hit>()
        fun add(title: String, subtitle: String?, target: NavTarget) {
            LauncherSearch.rank(tokens, title)?.let { out += Hit(title, subtitle, target, it) }
        }
        for (device in data.devices) add(device.name, StateTexts.text(device.status, device.lan), SeActions.tapDevice(device.status, device.location))
        for (room in data.rooms) {
            // A room opens a popup on the home screen; a hit in the search has no place for it, so it opens the Share tab.
            add(room.name, "Raum · " + SeViews.membersText(room), SeActions.SHARE_TAB)
            for (member in room.members) {
                add(member.name, room.name + " · " + StateTexts.text(member.status), SeActions.tapDevice(member.status, member.location))
            }
        }
        for (vault in data.vaults) add(vault.label, StateTexts.vault(vault.state), SeActions.tapVault(vault.site))
        add(SeViews.ALL_SCREENS, null, SeActions.screenGrid(null))
        return out
    }
}
