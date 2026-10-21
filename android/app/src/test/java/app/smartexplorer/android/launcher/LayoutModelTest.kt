package app.smartexplorer.android.launcher

import app.smartexplorer.android.launcher.model.Cell
import app.smartexplorer.android.launcher.model.DropOutcome
import app.smartexplorer.android.launcher.model.DropPlanner
import app.smartexplorer.android.launcher.model.GridSpec
import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.model.HomeLayout
import app.smartexplorer.android.launcher.model.Kinds
import app.smartexplorer.android.launcher.model.LayoutJson
import app.smartexplorer.android.launcher.model.LayoutOps
import app.smartexplorer.android.launcher.model.LayoutReadResult
import app.smartexplorer.android.launcher.model.Span
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** `layout.json` and the rules of folders and drops (U35-18, U35-22, U35-23), without Android. */
class LayoutModelTest {
    private val source = """
        {"schema":1,"grid":{"columns":5,"rows":6,"iconDp":56},
         "pages":[{"items":[
           {"id":"a","kind":"app","cell":[0,0],"span":[1,1],"component":"p.one/p.one.Main","user":0},
           {"id":"w","kind":"widget","cell":[1,0],"span":[2,2],"provider":"p.w/p.w.Provider","extraKey":{"x":[1,2]}},
           {"id":"f","kind":"folder","cell":[3,0],"span":[1,1],"name":"Spiele","children":[
              {"id":"c1","kind":"app","component":"p.g1/p.g1.Main","user":0},
              {"id":"c2","kind":"app","component":"p.g2/p.g2.Main","user":0}]},
           {"id":"z","kind":"seFuture","cell":[0,1],"span":[1,1],"target":"vault://site/","more":true}
         ]},{"items":[]}],
         "dock":[{"id":"d","kind":"app","cell":[0,0],"component":"p.d/p.d.Main","user":10}],
         "seSeeded":true,"showPrivateSpace":false,"zFuture":{"k":1}}
    """.trimIndent()

    private fun loaded(text: String): HomeLayout = (LayoutJson.parse(text) as LayoutReadResult.Loaded).layout

    private fun app(id: String, x: Int, y: Int, pkg: String = "p.$id") = HomeItem(
        id,
        Kinds.APP,
        Cell(x, y),
        attrs = JsonObject(mapOf("component" to JsonPrimitive("$pkg/$pkg.Main"), "user" to JsonPrimitive(0))),
    )

    private fun layout(vararg items: HomeItem, columns: Int = 4, rows: Int = 4) =
        HomeLayout(GridSpec(columns, rows, 56), listOf(items.toList()), emptyList())

    @Test
    fun theFileRoundTripsWithUnknownKeysAndKinds() {
        val first = loaded(source)
        val encoded = LayoutJson.encode(first)
        assertEquals(first, loaded(encoded))
        // Fixed key order, the unknown top-level keys last and sorted.
        assertTrue(encoded, encoded.startsWith("{\"schema\":1,\"grid\":{\"columns\":5,\"rows\":6,\"iconDp\":56},\"pages\":["))
        assertTrue(encoded.indexOf("\"dock\"") < encoded.indexOf("\"seSeeded\""))
        assertTrue(encoded.indexOf("\"seSeeded\"") < encoded.indexOf("\"showPrivateSpace\""))
        assertTrue(encoded.indexOf("\"showPrivateSpace\"") < encoded.indexOf("\"zFuture\""))
        // Unknown kinds and keys stay as read.
        val unknown = first.pages[0].first { it.id == "z" }
        assertEquals("seFuture", unknown.kind)
        assertEquals("vault://site/", unknown.attr("target"))
        assertTrue(encoded.contains("\"more\":true"))
        assertTrue(encoded.contains("\"extraKey\":{\"x\":[1,2]}"))
        assertEquals(Span(2, 2), first.pages[0].first { it.id == "w" }.span)
        assertEquals(JsonPrimitive(true), first.extra["seSeeded"])
        // Folder children carry no cell or span when written.
        val folder = first.pages[0].first { it.id == "f" }
        assertEquals(listOf("c1", "c2"), folder.children.map { it.id })
        assertFalse(encoded.substringAfter("\"children\":").substringBefore("]").contains("\"cell\""))
    }

    @Test
    fun aSecondEntryWithTheSameIdIsRenumbered() {
        val text = """{"schema":1,"pages":[{"items":[
            {"id":"x","kind":"app","cell":[0,0]},{"id":"x","kind":"app","cell":[1,0]},{"id":"x","kind":"app","cell":[2,0]}]}],"dock":[{"id":"x","kind":"app"}]}"""
        val layout = loaded(text)
        assertEquals(listOf("x", "x-2", "x-3"), layout.pages[0].map { it.id })
        assertEquals("x-4", layout.dock[0].id)
        // A missing span is 1x1, a cell outside the grid is kept as read.
        assertEquals(Span(1, 1), layout.pages[0][0].span)
        assertEquals(Cell(0, 0), layout.dock[0].cell)
    }

    @Test
    fun anotherSchemaOrBrokenJsonIsCorrupt() {
        assertTrue(LayoutJson.parse(source.replace("\"schema\":1", "\"schema\":2")) is LayoutReadResult.Corrupt)
        assertTrue(LayoutJson.parse("""{"pages":[]}""") is LayoutReadResult.Corrupt)
        assertTrue(LayoutJson.parse("{") is LayoutReadResult.Corrupt)
        assertTrue(LayoutJson.parse("[1]") is LayoutReadResult.Corrupt)
    }

    @Test
    fun newIdsLookLikeSpecifiedAndAvoidCollisions() {
        val id = LayoutJson.newId(emptySet())
        assertTrue(id, Regex("i[0-9a-f]{12}").matches(id))
        // A taken id is never given out again.
        assertTrue(id != LayoutJson.newId(setOf(id)))
    }

    @Test
    fun aFolderWithOneChildBecomesTheChildAndOneWithNoneVanishes() {
        val folder = loaded(source).pages[0].first { it.id == "f" }
        val base = layout(folder, app("other", 0, 3))
        val one = LayoutOps.remove(base, "c1")
        val child = one.pages[0].first { it.id == "c2" }
        assertEquals(Cell(3, 0), child.cell)
        assertEquals(Kinds.APP, child.kind)
        assertTrue(one.pages[0].none { it.id == "f" })
        val none = LayoutOps.remove(one, "c2")
        assertEquals(listOf("other"), none.pages[0].map { it.id })
        // Removing a package takes its folder children along and applies the same rule.
        val byPackage = LayoutOps.removePackage(base, "p.g1", 0L)
        assertEquals(listOf("c2", "other"), byPackage.pages[0].map { it.id })
    }

    @Test
    fun droppingAnAppOnAnAppMakesAFolderAndOnAFolderAppendsInOrder() {
        val base = layout(app("a", 0, 0), app("b", 1, 0), app("c", 2, 0))
        val outcome = DropPlanner.dropOnPage(base, app("b", 1, 0), 0, Cell(0, 0), merge = true) as DropOutcome.Applied
        val folder = outcome.layout.pages[0].first { it.isFolder }
        assertEquals(Cell(0, 0), folder.cell)
        assertEquals("Ordner", folder.name)
        assertEquals(listOf("a", "b"), folder.children.map { it.id })
        assertTrue(outcome.layout.pages[0].none { it.id == "b" })
        val appended = DropPlanner.dropOnPage(outcome.layout, app("c", 2, 0), 0, Cell(0, 0), merge = true) as DropOutcome.Applied
        assertEquals(listOf("a", "b", "c"), appended.layout.pages[0].first { it.isFolder }.children.map { it.id })
        // Without the dwell time the drop goes to the nearest free cell instead.
        val plain = DropPlanner.dropOnPage(base, app("c", 2, 0), 0, Cell(0, 0), merge = false) as DropOutcome.Applied
        assertTrue(plain.layout.pages[0].none { it.isFolder })
        assertEquals(Cell(0, 1), plain.layout.pages[0].first { it.id == "c" }.cell)
    }

    @Test
    fun aFullPageRefusesTheDropAndNamingAFolderKeepsAnEmptyNameOut() {
        val items = (0 until 4).flatMap { y -> (0 until 4).map { x -> app("i$x$y", x, y) } }
        val full = layout(*items.toTypedArray())
        assertEquals(DropOutcome.NoSpace, DropPlanner.dropOnPage(full, app("new", 0, 0, "p.new"), 0, Cell(1, 1), merge = false))
        // A new page takes it.
        val onNew = DropPlanner.dropOnPage(full, app("new", 0, 0, "p.new"), 1, Cell(2, 2), merge = false) as DropOutcome.Applied
        assertEquals(2, onNew.layout.pages.size)
        assertEquals(Cell(2, 2), onNew.layout.pages[1].single().cell)
        val folder = LayoutOps.createFolder(layout(app("a", 0, 0), app("b", 1, 0)), app("a", 0, 0), app("b", 1, 0))
        val id = folder.pages[0].first { it.isFolder }.id
        assertEquals("Ordner", LayoutOps.renameFolder(folder, id, "   ").pages[0].first { it.isFolder }.name)
        assertEquals("Spiele", LayoutOps.renameFolder(folder, id, " Spiele ").pages[0].first { it.isFolder }.name)
    }

    @Test
    fun dissolvingAFolderPutsTheChildrenOnTheNearestFreeCells() {
        val withoutB = layout(app("a", 0, 0))
        val folderLayout = LayoutOps.createFolder(withoutB, app("a", 0, 0), app("b", 1, 0))
        val folder = folderLayout.pages[0].first { it.isFolder }
        val result = LayoutOps.dissolveFolder(folderLayout, folder.id)
        assertTrue(result.pages[0].none { it.isFolder })
        assertEquals(setOf(Cell(0, 0), Cell(1, 0)), result.pages[0].map { it.cell }.toSet())
        assertEquals(setOf("a", "b"), result.pages[0].map { it.id }.toSet())
    }
}
