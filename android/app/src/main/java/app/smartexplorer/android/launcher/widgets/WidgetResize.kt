package app.smartexplorer.android.launcher.widgets

import android.appwidget.AppWidgetProviderInfo
import android.os.Build
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.launcher.model.Cell
import app.smartexplorer.android.launcher.model.GridGeometry
import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.model.Span
import app.smartexplorer.android.launcher.ui.HomeMetrics
import app.smartexplorer.android.launcher.ui.ItemContext
import kotlin.math.roundToInt

/** The limits of a widget's size in cells (U38-9): its provider's resize data and the grid [columns] x [rows]. */
fun resizeLimitsOf(info: AppWidgetProviderInfo, span: Span, cellWidthPx: Float, cellHeightPx: Float, columns: Int, rows: Int): ResizeLimits {
    val modern = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
    return WidgetResizing.limits(
        resizeMode = info.resizeMode,
        current = span,
        minResizeWidthPx = info.minResizeWidth,
        minResizeHeightPx = info.minResizeHeight,
        maxResizeWidthPx = if (modern) info.maxResizeWidth else 0,
        maxResizeHeightPx = if (modern) info.maxResizeHeight else 0,
        cellWidthPx = cellWidthPx,
        cellHeightPx = cellHeightPx,
        columns = columns,
        rows = rows,
    )
}

private val HANDLE_NAMES = mapOf(Side.Left to "links", Side.Top to "oben", Side.Right to "rechts", Side.Bottom to "unten")
private val HANDLE_TOUCH = 48.dp
private val HANDLE_VISIBLE = 14.dp

/**
 * The handles on the four sides of the widget chosen with „Größe ändern“ (U38-9), inside the box of the page area.
 * A side that the provider fixes has no handle. While a handle is pulled an outline shows the area that would
 * result; letting go stores it. „Fertig“ ends the mode. [others] are the other items of the page.
 */
@Composable
fun WidgetResizeOverlay(item: HomeItem, others: List<HomeItem>, metrics: HomeMetrics, context: ItemContext, onDone: () -> Unit) {
    val vm = context.vm
    val info = item.widgetId?.let { vm.widgets.manager.getAppWidgetInfo(it) } ?: return
    val limits = remember(item, info, metrics) { resizeLimitsOf(info, item.span, metrics.cellW, metrics.cellH, metrics.columns, metrics.rows) }
    val taken = remember(others, item.id) { GridGeometry.occupied(others.filter { it.id != item.id }) }
    var preview by remember(item) { mutableStateOf<Pair<Cell, Span>?>(null) }
    val density = LocalDensity.current
    val area = preview ?: (item.cell to item.span)
    val left = area.first.x * metrics.cellW
    val top = area.first.y * metrics.cellH
    val width = area.second.w * metrics.cellW
    val height = area.second.h * metrics.cellH
    val touch = with(density) { HANDLE_TOUCH.toPx() }

    Box(
        Modifier.offset { IntOffset(left.roundToInt(), top.roundToInt()) }
            .size(with(density) { width.toDp() }, with(density) { height.toDp() })
            .border(2.dp, MaterialTheme.colorScheme.primary, RoundedCornerShape(12.dp)),
    )
    for (side in Side.entries) {
        val enabled = if (side == Side.Left || side == Side.Right) limits.horizontal else limits.vertical
        if (!enabled) continue
        val centerX = when (side) {
            Side.Left -> left
            Side.Right -> left + width
            else -> left + width / 2
        }
        val centerY = when (side) {
            Side.Top -> top
            Side.Bottom -> top + height
            else -> top + height / 2
        }
        val horizontal = side == Side.Left || side == Side.Right
        val cellPx = if (horizontal) metrics.cellW else metrics.cellH
        Box(
            Modifier.offset { IntOffset((centerX - touch / 2).roundToInt(), (centerY - touch / 2).roundToInt()) }
                .size(HANDLE_TOUCH)
                .semantics { contentDescription = "Griff ${HANDLE_NAMES.getValue(side)}" }
                .pointerInput(item, limits, taken, side) {
                    var total = 0f
                    detectDragGestures(
                        onDragStart = { total = 0f },
                        onDrag = { change, amount ->
                            change.consume()
                            total += if (horizontal) amount.x else amount.y
                            preview = WidgetResizing.pull(item.cell, item.span, side, (total / cellPx).roundToInt(), limits, taken, metrics.columns, metrics.rows)
                        },
                        onDragEnd = {
                            preview?.let { vm.setWidgetArea(item.id, it.first, it.second) }
                            preview = null
                        },
                        onDragCancel = { preview = null },
                    )
                },
            contentAlignment = Alignment.Center,
        ) {
            Box(Modifier.size(HANDLE_VISIBLE).clip(RoundedCornerShape(50)).background(MaterialTheme.colorScheme.primary))
        }
    }
    Surface(
        Modifier.offset { IntOffset((left + width / 2 - with(density) { 40.dp.toPx() }).roundToInt(), (top + height - with(density) { 56.dp.toPx() }).roundToInt()) },
        shape = RoundedCornerShape(50),
        tonalElevation = 4.dp,
    ) { TextButton(onClick = onDone) { Text("Fertig") } }
}

/**
 * Accessibility alternatives to the handles (U35-32): „Breiter“, „Schmaler“, „Höher“, „Niedriger“ change the
 * span by one cell where the limits allow.
 */
fun widgetResizeActions(item: HomeItem, context: ItemContext): List<CustomAccessibilityAction> {
    val vm = context.vm
    return listOf(
        CustomAccessibilityAction("Breiter") { vm.stepWidget(item, horizontal = true, grow = true) },
        CustomAccessibilityAction("Schmaler") { vm.stepWidget(item, horizontal = true, grow = false) },
        CustomAccessibilityAction("Höher") { vm.stepWidget(item, horizontal = false, grow = true) },
        CustomAccessibilityAction("Niedriger") { vm.stepWidget(item, horizontal = false, grow = false) },
    )
}
