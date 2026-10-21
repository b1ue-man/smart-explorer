package app.smartexplorer.android.launcher

import app.smartexplorer.android.launcher.model.Cell
import app.smartexplorer.android.launcher.model.GridSpec
import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.model.HomeLayout
import app.smartexplorer.android.launcher.model.Kinds
import app.smartexplorer.android.launcher.model.LayoutJson
import app.smartexplorer.android.launcher.model.LayoutOps
import app.smartexplorer.android.launcher.model.LayoutReadResult
import app.smartexplorer.android.launcher.model.Span
import app.smartexplorer.android.launcher.widgets.PendingWidget
import app.smartexplorer.android.launcher.widgets.RestoreCase
import app.smartexplorer.android.launcher.widgets.WidgetRestore
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Widget items in the layout, the saved flow state and the restoration decision (U38-6, U38-7), without Android. */
class WidgetFlowTest {
    private fun widget(id: String, widgetId: Int?, provider: String = "p.w/p.w.Provider", user: Long = 0, configured: Boolean? = null) = HomeItem(
        id,
        Kinds.WIDGET,
        Cell(0, 0),
        Span(2, 2),
        JsonObject(
            buildMap {
                put("provider", JsonPrimitive(provider))
                put("user", JsonPrimitive(user))
                if (widgetId != null) put("widgetId", JsonPrimitive(widgetId))
                if (configured != null) put("configured", JsonPrimitive(configured))
            },
        ),
    )

    private fun layout(vararg items: HomeItem) = HomeLayout(GridSpec(4, 5, 56), listOf(items.toList()), emptyList())

    @Test
    fun aWidgetItemKeepsItsKeysThroughTheFile() {
        val before = layout(widget("w", 17, configured = false))
        val after = (LayoutJson.parse(LayoutJson.encode(before)) as LayoutReadResult.Loaded).layout
        assertEquals(before, after)
        val item = after.pages[0][0]
        assertTrue(item.isWidget)
        assertEquals(17, item.widgetId)
        assertEquals("p.w/p.w.Provider", item.provider)
        assertEquals("p.w", item.providerPackage)
        assertFalse(item.configured)
        // A missing flag means set up; a missing id means none.
        assertTrue(widget("x", null).configured)
        assertNull(widget("x", null).widgetId)
    }

    @Test
    fun everyWayOutOfTheArrangementShowsInTheIdSet() {
        val full = layout(widget("a", 1), widget("b", 2, provider = "q.w/q.w.P"), widget("c", null))
        assertEquals(setOf(1, 2), LayoutOps.widgetIds(full))
        // Package removal takes the widgets of that package and profile only.
        val removed = LayoutOps.removePackage(full, "p.w", 0)
        assertEquals(setOf(2), LayoutOps.widgetIds(removed))
        // "c" names the same provider and profile, so it goes too, bound or not.
        assertEquals(listOf("b"), removed.pages[0].map { it.id })
        val otherProfile = LayoutOps.removePackage(full, "p.w", 10)
        assertEquals(setOf(1, 2), LayoutOps.widgetIds(otherProfile))
        assertEquals(setOf(1), LayoutOps.widgetIds(LayoutOps.remove(full, "b")))
        // The new id and state of a restored widget go in without moving it.
        val rebound = LayoutOps.updateItem(full, "a") {
            it.withAttr("widgetId", JsonPrimitive(40)).withAttr("configured", JsonPrimitive(false))
        }
        assertEquals(setOf(40, 2), LayoutOps.widgetIds(rebound))
        assertEquals(full.pages[0][0].cell, rebound.pages[0][0].cell)
        assertFalse(rebound.pages[0][0].configured)
    }

    @Test
    fun theAreaOfAWidgetChangesAndNothingElse() {
        val full = layout(widget("a", 1))
        val moved = LayoutOps.setArea(full, "a", Cell(1, 2), Span(3, 1))
        assertEquals(Cell(1, 2), moved.pages[0][0].cell)
        assertEquals(Span(3, 1), moved.pages[0][0].span)
        assertEquals(full.pages[0][0].attrs, moved.pages[0][0].attrs)
    }

    @Test
    fun theSavedStateOfTheFlowRoundTrips() {
        val pending = PendingWidget(12, "p.w/p.w.Outer\$Provider", 10, Span(2, 3), PendingWidget.Stage.Configure, "i42")
        assertEquals(pending, PendingWidget.decode(pending.encode()))
        val adding = pending.copy(itemId = null, stage = PendingWidget.Stage.Bind)
        assertEquals(adding, PendingWidget.decode(adding.encode()))
        assertNull(PendingWidget.decode(null))
        assertNull(PendingWidget.decode(""))
        assertNull(PendingWidget.decode("12|noslash|0|1|1|Bind|"))
        assertNull(PendingWidget.decode("x|p/p.P|0|1|1|Bind|"))
        assertNull(PendingWidget.decode("12|p/p.P|0|1|1|Sideways|"))
        assertNull(PendingWidget.decode("12|p/p.P|0|1|1|Bind"))
    }

    @Test
    fun theRestorationDecidesInTheOrderOfTheFourSteps() {
        // 1: the id still names the provider.
        assertEquals(RestoreCase.Keep, WidgetRestore.classify("p/p.W", "p/p.W", installed = true, profileReachable = true))
        // 2: unknown id, provider installed: bind again.
        assertEquals(RestoreCase.Rebind, WidgetRestore.classify(null, "p/p.W", installed = true, profileReachable = true))
        // The id belongs to something else now: bind again as well.
        assertEquals(RestoreCase.Rebind, WidgetRestore.classify("q/q.W", "p/p.W", installed = true, profileReachable = true))
        // 4: the provider is gone, the profile could be asked: remove.
        assertEquals(RestoreCase.Remove, WidgetRestore.classify(null, "p/p.W", installed = false, profileReachable = true))
        // A profile that is locked or paused cannot be asked, so nothing is removed.
        assertEquals(RestoreCase.Keep, WidgetRestore.classify(null, "p/p.W", installed = false, profileReachable = false))
    }
}
