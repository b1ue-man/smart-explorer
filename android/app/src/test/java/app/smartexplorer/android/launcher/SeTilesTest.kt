package app.smartexplorer.android.launcher

import app.smartexplorer.android.launcher.bridge.BridgeProtocol
import app.smartexplorer.android.launcher.bridge.DeviceEntry
import app.smartexplorer.android.launcher.bridge.DevicesData
import app.smartexplorer.android.launcher.bridge.DevicesFolder
import app.smartexplorer.android.launcher.bridge.MemberEntry
import app.smartexplorer.android.launcher.bridge.RoomEntry
import app.smartexplorer.android.launcher.bridge.ScreenRef
import app.smartexplorer.android.launcher.bridge.StateTexts
import app.smartexplorer.android.launcher.bridge.Tone
import app.smartexplorer.android.launcher.bridge.VaultEntry
import app.smartexplorer.android.launcher.model.Cell
import app.smartexplorer.android.launcher.model.GridSpec
import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.model.HomeLayout
import app.smartexplorer.android.launcher.model.Kinds
import app.smartexplorer.android.launcher.model.LayoutOps
import app.smartexplorer.android.launcher.se.BridgeState
import app.smartexplorer.android.launcher.se.NavTarget
import app.smartexplorer.android.launcher.se.SeActions
import app.smartexplorer.android.launcher.se.SeSearchSource
import app.smartexplorer.android.launcher.se.SeSeeder
import app.smartexplorer.android.launcher.se.SeUiState
import app.smartexplorer.android.launcher.se.SeViews
import app.smartexplorer.android.launcher.se.SeVirtual
import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.MainTab
import app.smartexplorer.android.ui.NavRequest
import app.smartexplorer.android.ui.screen.ScreenTarget
import app.smartexplorer.android.ui.screen.ScreenTargetJson
import app.smartexplorer.android.widget.WidgetCache
import app.smartexplorer.android.widget.WidgetSnapshot
import app.smartexplorer.android.widget.WidgetTarget
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** DSK-M13 on the JVM without Android (U40-9 to U40-16, U40-21): tile texts, navigation, the first fill, search and the widget rows. */
class SeTilesTest {
    private val direct = DeviceEntry(
        "c1", "Laptop", "connectedDirect", "Direkt verbunden", true, "share://direct/c1", false, true, false, true, false,
        "direct~c1", ScreenRef("direct", "c1", "dev-c1"),
    )
    private val offline = direct.copy(contactId = "c2", name = "Tablet", status = "offline", statusText = "Offline", online = false, location = "share://direct/c2", key = "direct~c2", screen = null)
    private val member = MemberEntry("d1", "Desktop", "available", "Online", true, "share://room/p1/d1", false, "room~p1~d1", ScreenRef("room", "p1", "d1"))
    private val member2 = MemberEntry("d2", "Server", "waitingForAccess", "Warte auf Freigabe", false, "share://room/p1/d2", false, "room~p1~d2", null)
    private val room = RoomEntry("p1", "ro1", "Team", "Verbunden", listOf(member, member2))
    private val data = DevicesData(
        devices = listOf(direct, offline),
        rooms = listOf(room),
        vaults = listOf(VaultEntry("s1", "Privat", "locked")),
        devicesFolder = DevicesFolder("app.documents", "devices"),
    )
    private val live = SeUiState(data, BridgeState.Connected)

    private fun item(kind: String, target: String?, name: String? = null, id: String = "i1") = HomeItem(
        id = id,
        kind = kind,
        attrs = JsonObject(
            buildMap {
                target?.let { put("target", JsonPrimitive(it)) }
                name?.let { put("name", JsonPrimitive(it)) }
            },
        ),
    )

    private fun layout(vararg items: HomeItem, seeded: Boolean? = null, columns: Int = 4, rows: Int = 5) = HomeLayout(
        GridSpec(columns, rows, 56),
        listOf(items.toList()),
        emptyList(),
        JsonObject(if (seeded == null) emptyMap() else mapOf("seSeeded" to JsonPrimitive(seeded))),
    )

    // ---- texts (U40-9) -----------------------------------------------------------------------------

    @Test
    fun launcher_task_tile_state_texts() {
        val expected = listOf(
            "unknown" to ("Status unbekannt – nicht verbunden" to Tone.Neutral),
            "offline" to ("Offline" to Tone.Gray),
            "waiting" to ("Wartet auf das Gerät" to Tone.Gray),
            "waitingForAccess" to ("Warte auf Freigabe" to Tone.Yellow),
            "available" to ("Erreichbar" to Tone.OutlineGreen),
            "connecting" to ("Verbindet" to Tone.Progress),
            "connected" to ("Verbunden" to Tone.Green),
            "connectedDirect" to ("Verbunden (direkt)" to Tone.Green),
            "connectedRelay" to ("Verbunden (Relay)" to Tone.Green),
            "failed" to ("Fehler" to Tone.Red),
            "identityConflict" to ("Identität geändert – prüfen" to Tone.Red),
        )
        for ((code, pair) in expected) {
            assertEquals(code, pair.first, StateTexts.text(code))
            assertEquals(code, pair.second, StateTexts.tone(code))
        }
        assertEquals("Erreichbar im LAN", StateTexts.text("available", lan = true))
        assertEquals("Offline · nicht aktuell", StateTexts.withStale("Offline", true))
        assertEquals("Offline", StateTexts.withStale("Offline", false))
        assertEquals("Status unbekannt – nicht verbunden", StateTexts.text("eine-neue-art"))
        assertEquals(listOf("Gesperrt", "Entsperrt", "Nicht verfügbar"), listOf("locked", "unlocked", "unavailable").map { StateTexts.vault(it) })
    }

    @Test
    fun a_device_tile_shows_its_state_and_a_failure_with_its_reason() {
        val view = SeViews.device(item(Kinds.SE_DEVICE, "share://direct/c1"), live)
        assertEquals("Laptop", view.title)
        assertEquals("Verbunden (direkt)", view.subtitle)
        assertEquals("Laptop, Verbunden (direkt)", view.description)
        val failed = data.copy(devices = listOf(direct.copy(status = "failed", statusText = "Fehler: Zeitüberschreitung")))
        val text = SeViews.device(item(Kinds.SE_DEVICE, "share://direct/c1"), SeUiState(failed, BridgeState.Connected)).description
        assertEquals("Laptop, Fehler, Zeitüberschreitung", text)
    }

    @Test
    fun cached_data_carry_not_current() {
        val stale = SeUiState(data.copy(stale = true), BridgeState.Idle)
        assertFalse(stale.fresh)
        assertEquals("Offline · nicht aktuell", SeViews.device(item(Kinds.SE_DEVICE, "share://direct/c2"), stale).subtitle)
        assertEquals("2 Geräte · nicht aktuell", SeViews.room(item(Kinds.SE_ROOM, "p1"), stale).subtitle)
        assertEquals("Gesperrt · nicht aktuell", SeViews.vault(item(Kinds.SE_VAULT, "vault://s1/"), stale).subtitle)
    }

    @Test
    fun a_target_that_left_the_current_data_shows_gone_but_stale_data_leave_it_alone() {
        val gone = item(Kinds.SE_DEVICE, "share://direct/zzz", "Altes Gerät")
        val view = SeViews.device(gone, live)
        assertTrue(view.gone)
        assertEquals("Altes Gerät", view.title)
        assertEquals("Nicht mehr vorhanden", view.subtitle)
        // Without current data the tile stays as it is: it does not claim the device is gone.
        val cached = SeUiState(data.copy(stale = true), BridgeState.Idle)
        assertFalse(SeViews.device(gone, cached).gone)
        assertEquals("Status unbekannt – nicht verbunden", SeViews.device(gone, cached).subtitle)
        assertTrue(SeViews.vault(item(Kinds.SE_VAULT, "vault://weg/", "Alt"), live).gone)
        assertTrue(SeViews.room(item(Kinds.SE_ROOM, "weg", "Raum alt"), live).gone)
    }

    @Test
    fun a_tile_shows_connecting_while_the_binding_waits() {
        val view = SeViews.device(item(Kinds.SE_DEVICE, "share://direct/c1"), SeUiState(data.copy(stale = true), BridgeState.Binding))
        assertEquals("Verbindet", view.subtitle)
        assertTrue(view.progress)
        assertEquals(Tone.Progress, view.tone)
        assertEquals("1 Gerät", SeViews.membersText(room.copy(members = listOf(member))))
    }

    // ---- navigation (U40-11, U40-12) -------------------------------------------------------------------

    @Test
    fun launcher_task_screen_nav_kinds() {
        val json = ScreenTarget("direct", "c1", "dev-c1").toJson()
        assertEquals(NavRequest.OpenScreen(json), AppNav.decode("openScreen", json))
        assertEquals(NavRequest.ShowScreenGrid(null), AppNav.decode("showScreenGrid", ""))
        assertEquals(NavRequest.ShowScreenGrid("room-1"), AppNav.decode("showScreenGrid", "room-1"))
        assertEquals("openScreen" to json, AppNav.encode(NavRequest.OpenScreen(json)))
        assertEquals("showScreenGrid" to "", AppNav.encode(NavRequest.ShowScreenGrid(null)))
        assertEquals("showScreenGrid" to "room-1", AppNav.encode(NavRequest.ShowScreenGrid("room-1")))
        assertEquals(MainTab.Share, AppNav.tabFor(NavRequest.OpenScreen(json)))
        assertEquals(MainTab.Share, AppNav.tabFor(NavRequest.ShowScreenGrid("room-1")))
    }

    @Test
    fun taps_open_the_folder_or_the_share_tab() {
        assertEquals(NavTarget("openLocation", "share://direct/c1/"), SeActions.tapDevice("connectedDirect", "share://direct/c1"))
        assertEquals(NavTarget("openLocation", "share://room/p1/d1/"), SeActions.tapDevice("offline", "share://room/p1/d1/"))
        assertEquals(NavTarget("openLocation", "share://direct/c1/"), SeActions.tapDevice("unknown", "share://direct/c1"))
        assertEquals(NavTarget("selectTab", "Share"), SeActions.tapDevice("waitingForAccess", "share://direct/c1"))
        assertEquals(NavTarget("selectTab", "Share"), SeActions.tapDevice("identityConflict", "share://direct/c1"))
        assertEquals(NavTarget("openLocation", "vault://s1/"), SeActions.tapVault("s1"))
        assertEquals(NavTarget("showScreenGrid", ""), SeActions.screenGrid())
        assertEquals(NavTarget("showScreenGrid", "p1"), SeActions.screenGrid("p1"))
        // Every target decodes to a request of the app.
        assertEquals(NavRequest.SelectTab(MainTab.Share), AppNav.decode(SeActions.SHARE_TAB.kind, SeActions.SHARE_TAB.value))
        assertEquals(NavRequest.OpenLocation("share://direct/c1/"), SeActions.tapDevice("connected", "share://direct/c1").let { AppNav.decode(it.kind, it.value) })
    }

    @Test
    fun the_screen_target_json_matches_the_one_of_the_main_process() {
        val ref = ScreenRef("room", "p1", "d1")
        for (control in listOf(false, true)) {
            val built = SeActions.screenJson(ref, control)
            val parsed = ScreenTargetJson.parse(built).getOrThrow()
            assertEquals(ScreenTarget("room", "p1", "d1", null, control), parsed)
            assertEquals(ScreenTarget("room", "p1", "d1", null, control).toJson(), built)
        }
        assertEquals(NavTarget("openScreen", SeActions.screenJson(ref, true)), SeActions.openScreen(ref, true))
    }

    // ---- the first fill and the sheet (U40-10) ------------------------------------------------------------

    @Test
    fun the_first_fill_adds_devices_rooms_vaults_and_all_screens_in_that_order() {
        val seeded = SeSeeder.seed(layout(seeded = false), data)
        assertTrue(SeSeeder.isSeeded(seeded))
        val items = seeded.pages[0]
        assertEquals(
            listOf(Kinds.SE_DEVICE, Kinds.SE_DEVICE, Kinds.SE_ROOM, Kinds.SE_VAULT, Kinds.SE_SCREENS),
            items.map { it.kind },
        )
        assertEquals(listOf("share://direct/c1", "share://direct/c2", "p1", "vault://s1/", null), items.map { it.attr("target") })
        assertEquals(listOf("Laptop", "Tablet", "Team", "Privat", "Alle Bildschirme"), items.map { it.attr("name") })
        // Reading order from the first free cell.
        assertEquals(listOf(Cell(0, 0), Cell(1, 0), Cell(2, 0), Cell(3, 0), Cell(0, 1)), items.map { it.cell })
        assertEquals(items.size, LayoutOps.allItems(seeded).map { it.id }.toSet().size)
    }

    @Test
    fun the_first_fill_overflows_to_the_next_page_and_keeps_other_items() {
        val app = HomeItem("a1", Kinds.APP, Cell(0, 0))
        val seeded = SeSeeder.seed(layout(app, columns = 2, rows = 1), data)
        assertTrue(seeded.pages.size >= 3)
        assertEquals("a1", seeded.pages[0].first { it.isApp }.id)
        assertEquals(6, LayoutOps.allItems(seeded).size)
    }

    @Test
    fun adding_from_the_sheet_skips_what_is_on_a_page_and_never_seeds_again() {
        val start = SeSeeder.seed(layout(seeded = false), data)
        val removed = LayoutOps.remove(start, start.pages[0].first { it.kind == Kinds.SE_VAULT }.id)
        assertTrue(SeSeeder.isSeeded(removed))
        val again = SeSeeder.add(removed, SeSeeder.entries(data), 0)
        // Only the removed vault returns; the others were present already.
        assertEquals(start.pages[0].size, again.pages[0].size)
        assertEquals(setOf("seDevice", "seRoom", "seVault", "seScreens"), again.pages[0].map { it.kind }.toSet())
        assertEquals(SeSeeder.present(start), SeSeeder.present(again))
        // A device that appears later is offered by the sheet, not added by itself.
        val later = data.copy(devices = data.devices + direct.copy(contactId = "c3", name = "Neu", location = "share://direct/c3"))
        assertEquals(6, SeSeeder.entries(later).size)
        assertFalse(SeSeeder.present(again).contains(SeSeeder.entries(later)[2].key))
    }

    @Test
    fun the_stored_name_follows_the_data_without_a_write_when_unchanged() {
        val start = SeSeeder.seed(layout(seeded = false), data)
        assertTrue(SeSeeder.syncNames(start, data) === start)
        val renamed = data.copy(devices = listOf(direct.copy(name = "Laptop neu"), offline))
        val synced = SeSeeder.syncNames(start, renamed)
        assertEquals("Laptop neu", synced.pages[0][0].attr("name"))
        assertEquals("share://direct/c1", synced.pages[0][0].attr("target"))
    }

    @Test
    fun the_two_groups_of_an_empty_list_stand_on_the_first_free_cells_and_are_computed() {
        val empty = DevicesData()
        assertTrue(SeVirtual.wanted(empty, fresh = true, seeded = true))
        assertFalse(SeVirtual.wanted(empty, fresh = false, seeded = true))
        assertFalse(SeVirtual.wanted(empty, fresh = true, seeded = false))
        assertFalse(SeVirtual.wanted(data, fresh = true, seeded = true))
        val pages = listOf(listOf(HomeItem("a1", Kinds.APP, Cell(0, 0))), emptyList())
        val shown = SeVirtual.inject(pages, 4, 5)
        assertEquals(listOf(Kinds.APP, Kinds.SE_ADD_DEVICE, Kinds.SE_JOIN_ROOM), shown[0].map { it.kind })
        assertEquals(listOf(Cell(0, 0), Cell(1, 0), Cell(2, 0)), shown[0].map { it.cell })
        assertEquals(pages[1], shown[1])
        // The input is untouched: nothing is stored.
        assertEquals(1, pages[0].size)
        // A full page gets nothing.
        val full = listOf(listOf(HomeItem("a", Kinds.APP, Cell(0, 0))))
        assertEquals(full, SeVirtual.inject(full, 1, 1))
    }

    // ---- search (U40-15) --------------------------------------------------------------------------------

    @Test
    fun the_search_finds_devices_members_rooms_vaults_and_all_screens() {
        fun titles(query: String) = SeSearchSource.hits(data, app.smartexplorer.android.launcher.search.LauncherSearch.tokens(query)).map { it.title }
        assertEquals(listOf("Laptop"), titles("lap"))
        assertEquals(listOf("Desktop"), titles("desk"))
        assertEquals(listOf("Team"), titles("team"))
        assertEquals(listOf("Privat"), titles("priv"))
        assertEquals(listOf("Alle Bildschirme"), titles("bildsch"))
        assertTrue(titles("zzz").isEmpty())
        val hit = SeSearchSource.hits(data, listOf("server")).single()
        assertEquals(NavTarget("selectTab", "Share"), hit.target)
        assertEquals("Team · Warte auf Freigabe", hit.subtitle)
        assertEquals(NavTarget("openLocation", "share://direct/c1/"), SeSearchSource.hits(data, listOf("laptop")).single().target)
    }

    // ---- the widget (U40-16) -------------------------------------------------------------------------------

    @Test
    fun the_widget_rows_list_direct_devices_then_room_devices() {
        val rows = WidgetCache.rowsOf(data)
        assertEquals(listOf("Laptop", "Tablet", "Team · Desktop", "Team · Server"), rows.map { it.name })
        assertEquals(listOf("Verbunden (direkt)", "Offline", "Erreichbar", "Warte auf Freigabe"), rows.map { it.state })
        assertEquals(listOf(true, false, true, false), rows.map { it.online })
        assertEquals(WidgetTarget.OpenLocation("share://direct/c1"), rows[0].target)
        assertEquals(WidgetTarget.SelectTab("Share"), rows[3].target)
        assertEquals("openLocation" to "share://room/p1/d1/", rows[2].target.wire())
    }

    @Test
    fun the_widget_cache_round_trips_and_a_foreign_row_is_dropped() {
        val snapshot = WidgetSnapshot(WidgetCache.rowsOf(data), "tok")
        assertEquals(snapshot.rows.map { it.name }, WidgetCache.parse(WidgetCache.encode(snapshot))!!.rows.map { it.name })
        assertEquals("tok", WidgetCache.parse(WidgetCache.encode(snapshot))!!.token)
        val parsed = WidgetCache.parse(
            """{"token":null,"rows":[{"name":"x","state":"s","online":true,"target":{"kind":"neu","value":"v"}},
               {"name":"y","state":"s","online":false,"target":{"kind":"selectTab","value":"Share"}}]}""",
        )!!
        assertNull(parsed.token)
        assertEquals(listOf("y"), parsed.rows.map { it.name })
        assertNull(WidgetCache.parse("[]"))
        assertNotNull(WidgetCache.parse("""{"rows":[]}"""))
    }

    @Test
    fun launcher_task_widget_nav_constants_match() {
        // The widget code carries copies of the AppNav constants so it needs no AppNav (U40-16); they must stay equal.
        fun constant(name: String): String {
            val field = AppNav::class.java.getDeclaredField(name)
            field.isAccessible = true
            return field.get(null) as String
        }
        assertEquals(constant("ACTION_NAVIGATE"), BridgeProtocol.NAV_ACTION)
        assertEquals(constant("EXTRA_KIND"), BridgeProtocol.NAV_EXTRA_KIND)
        assertEquals(constant("EXTRA_VALUE"), BridgeProtocol.NAV_EXTRA_VALUE)
        assertEquals(constant("EXTRA_TOKEN"), BridgeProtocol.NAV_EXTRA_TOKEN)
        // The wire kinds of the rows decode to requests.
        for (row in WidgetCache.rowsOf(data)) {
            val (kind, value) = row.target.wire()
            assertNotNull(AppNav.decode(kind, value))
        }
    }
}
