package app.smartexplorer.android.launcher.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import app.smartexplorer.android.launcher.model.HomeItem

/**
 * The dock (U35-21): one row with as many places as the grid has columns, each place as wide as a grid
 * cell. [exclude] is the id of the item being dragged.
 */
@Composable
fun Dock(items: List<HomeItem>, metrics: HomeMetrics, context: ItemContext, exclude: String?, modifier: Modifier = Modifier) {
    val density = LocalDensity.current
    val cellW = with(density) { metrics.cellW.toDp() }
    val height = with(density) { metrics.dockHeight.toDp() }
    Box(modifier.fillMaxSize()) {
        for (item in items) {
            if (item.id == exclude) continue
            Box(Modifier.offset(x = cellW * item.cell.x).size(cellW, height)) { ItemContent(item, context) }
        }
    }
}
