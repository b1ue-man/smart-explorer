package app.smartexplorer.android.launcher.widgets

import android.appwidget.AppWidgetHostView
import android.appwidget.AppWidgetProviderInfo
import android.content.Context
import android.os.Build
import android.os.Bundle
import android.util.SizeF
import android.view.MotionEvent
import android.view.ViewConfiguration
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import app.smartexplorer.android.launcher.MenuSource
import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.ui.DragOrigin
import app.smartexplorer.android.launcher.ui.ItemContext
import app.smartexplorer.android.launcher.ui.cellSemantics
import kotlin.math.hypot

/** What a long press on a widget and the moves after it report; positions are local to the widget view. */
interface WidgetTouchSink {
    fun onLongPress(local: Offset)

    fun onMove(local: Offset)

    fun onEnd(local: Offset, cancelled: Boolean)
}

/**
 * The view the widget host makes (U38-4). It lets every touch through to the widget, except one that stays
 * within the touch slop until the long-press time has passed: that one is the launcher's — it opens the menu or
 * begins the drag (U35-22) — and from then on the view swallows the events and reports them to the [sink]. The
 * swipe-up gesture of the home screen never sees a touch on a widget.
 */
class LauncherWidgetHostView(context: Context) : AppWidgetHostView(context) {
    var sink: WidgetTouchSink? = null
    private var longPressed = false
    private var downX = 0f
    private var downY = 0f
    private val slop = ViewConfiguration.get(context).scaledTouchSlop
    private val fire = Runnable {
        longPressed = true
        sink?.onLongPress(Offset(downX, downY))
    }

    override fun onInterceptTouchEvent(ev: MotionEvent): Boolean {
        track(ev)
        return longPressed
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        track(event)
        if (!longPressed) return super.onTouchEvent(event)
        val at = Offset(event.x, event.y)
        when (event.actionMasked) {
            MotionEvent.ACTION_MOVE -> sink?.onMove(at)
            MotionEvent.ACTION_UP -> finish(at, cancelled = false)
            MotionEvent.ACTION_CANCEL -> finish(at, cancelled = true)
        }
        return true
    }

    private fun finish(at: Offset, cancelled: Boolean) {
        longPressed = false
        sink?.onEnd(at, cancelled)
    }

    private fun track(ev: MotionEvent) {
        when (ev.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                longPressed = false
                downX = ev.x
                downY = ev.y
                removeCallbacks(fire)
                postDelayed(fire, ViewConfiguration.getLongPressTimeout().toLong())
            }
            MotionEvent.ACTION_MOVE -> if (!longPressed && hypot(ev.x - downX, ev.y - downY) > slop) removeCallbacks(fire)
            MotionEvent.ACTION_POINTER_DOWN, MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> if (!longPressed) removeCallbacks(fire)
        }
    }

    override fun onDetachedFromWindow() {
        removeCallbacks(fire)
        super.onDetachedFromWindow()
    }
}

/** What a widget cell shows (U38-6). */
internal sealed interface WidgetShow {
    data class Live(val info: AppWidgetProviderInfo) : WidgetShow

    /** Configuration still to do: the card with „Einrichten“. */
    data object Setup : WidgetShow

    /** The id is unknown to the system and the approval is missing: the card with „Erlauben“. */
    data object Allow : WidgetShow
}

internal fun widgetShow(item: HomeItem, flow: WidgetFlow): WidgetShow {
    val info = item.widgetId?.let { flow.manager.getAppWidgetInfo(it) }
    return when {
        info == null || info.provider.flattenToString() != item.provider -> WidgetShow.Allow
        !item.configured -> WidgetShow.Setup
        else -> WidgetShow.Live(info)
    }
}

/** Tells the provider the size of its area in both orientations (U38-9): API 31 with a list, API 30 with four numbers. */
fun reportWidgetSize(view: AppWidgetHostView, sizes: Pair<SizeDp, SizeDp>) {
    val (portrait, landscape) = sizes
    try {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            view.updateAppWidgetSize(Bundle(), listOf(SizeF(portrait.width, portrait.height), SizeF(landscape.width, landscape.height)))
        } else {
            val v = WidgetSizes.fourValues(portrait, landscape)
            @Suppress("DEPRECATION")
            view.updateAppWidgetSize(Bundle(), v[0], v[1], v[2], v[3])
        }
    } catch (_: RuntimeException) {
        // The widget is being removed; nothing to tell.
    }
}

/**
 * A widget item on the home screen (U38-4, U38-6): the widget itself, or a card when it needs the person.
 * A widget being dragged stays in the composition, invisible, so the view that began the drag keeps its touches.
 */
@Composable
fun WidgetView(item: HomeItem, context: ItemContext) {
    val vm = context.vm
    val flow = vm.widgets
    val show = remember(item.widgetId, item.provider, item.configured, item.userSerial) { widgetShow(item, flow) }
    when (show) {
        is WidgetShow.Live -> LiveWidget(item, show.info, context)
        WidgetShow.Setup -> WidgetCard(flow.appLabel(item), "Einrichten", item, context) { vm.setUpWidget(item) }
        WidgetShow.Allow -> WidgetCard("${flow.appLabel(item)}: Widget erlauben", "Erlauben", item, context) { vm.allowWidget(item) }
    }
}

@Composable
private fun LiveWidget(item: HomeItem, info: AppWidgetProviderInfo, context: ItemContext) {
    val vm = context.vm
    val id = item.widgetId ?: return
    var view by remember(id) { mutableStateOf<LauncherWidgetHostView?>(null) }
    var position by remember { mutableStateOf(Offset.Zero) }
    var size by remember { mutableStateOf(IntSize.Zero) }
    val sink = remember(item, context) {
        object : WidgetTouchSink {
            private var started = false
            private val center get() = position + Offset(size.width / 2f, size.height / 2f)

            override fun onLongPress(local: Offset) {
                val host = context.dragHost ?: return
                if (context.isDragActive()) return
                started = true
                host.begin(item, position + local, DragOrigin.Home, MenuSource.Home, center)
            }

            override fun onMove(local: Offset) {
                if (started) context.dragHost?.move(position + local)
            }

            override fun onEnd(local: Offset, cancelled: Boolean) {
                if (!started) return
                started = false
                context.dragHost?.end(position + local, cancelled)
            }
        }
    }
    Box(
        Modifier.fillMaxSize()
            .onGloballyPositioned {
                position = it.positionInRoot()
                size = it.size
            }
            .cellSemantics(item, context),
    ) {
        key(id) {
            AndroidView(
                factory = { ctx ->
                    val created = vm.widgets.host.createView(ctx, id, info) as LauncherWidgetHostView
                    view = created
                    created
                },
                modifier = Modifier.fillMaxSize(),
                update = { it.sink = sink },
            )
        }
    }
    val grids = vm.sizeGrids
    LaunchedEffect(view, item.span, grids) {
        val v = view ?: return@LaunchedEffect
        val g = grids ?: return@LaunchedEffect
        reportWidgetSize(v, WidgetSizes.forSpan(item.span, g.portrait, g.landscape))
    }
}

/** The card for a widget that needs the person: the app's name (or the question) and one button. */
@Composable
private fun WidgetCard(title: String, button: String, item: HomeItem, context: ItemContext, onClick: () -> Unit) {
    Surface(
        Modifier.fillMaxSize().padding(2.dp).cellSemantics(item, context),
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.surface.copy(alpha = 0.85f),
    ) {
        Column(Modifier.padding(8.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center) {
            Text(title, style = MaterialTheme.typography.labelLarge, textAlign = TextAlign.Center, maxLines = 3, overflow = TextOverflow.Ellipsis)
            Button(onClick = onClick, modifier = Modifier.padding(top = 8.dp)) { Text(button) }
        }
    }
}
