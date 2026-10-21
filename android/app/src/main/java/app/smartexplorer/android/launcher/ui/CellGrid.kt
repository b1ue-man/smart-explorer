package app.smartexplorer.android.launcher.ui

import android.content.pm.ShortcutInfo
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Shadow
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.smartexplorer.android.launcher.LauncherViewModel
import app.smartexplorer.android.launcher.apps.AppEntry
import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.model.Kinds

/**
 * What the tiles need to draw: the view model, the symbol size and the apps and pinned shortcuts by
 * their keys. One instance per composition of the home screen.
 */
@Stable
class ItemContext(
    val vm: LauncherViewModel,
    val iconDp: Int,
    val apps: Map<String, AppEntry>,
    val shortcutInfos: Map<String, ShortcutInfo>,
) {
    fun entry(item: HomeItem): AppEntry? = item.component?.let { apps["$it#${item.userSerial}"] }

    fun shortcut(item: HomeItem): ShortcutInfo? = shortcutInfos[vm.shortcutKey(item)]

    /** Name shown under the tile and read by the accessibility service. */
    fun label(item: HomeItem): String = when (item.kind) {
        Kinds.APP -> entry(item)?.label ?: item.packageName.orEmpty()
        Kinds.SHORTCUT -> shortcut(item)?.shortLabel?.toString() ?: "Verknüpfung"
        Kinds.FOLDER -> item.name
        else -> "Unbekanntes Element"
    }
}

/** The label color on the wallpaper: white with a shadow, as home screens do. */
private val LabelColor = Color.White
private val LabelShadow = Shadow(color = Color(0xCC000000), offset = androidx.compose.ui.geometry.Offset(0f, 2f), blurRadius = 6f)

/**
 * Extension point of the kinds (U35-31): U38 adds `"widget"`, U40 `"seDevice"`, `"seRoom"`, `"seVault"` and
 * `"seScreens"`, each with one branch and one file. An unknown kind stays in the file and occupies its cell.
 */
@Composable
fun ItemContent(item: HomeItem, context: ItemContext) {
    when (item.kind) {
        Kinds.APP -> AppTile(item, context)
        Kinds.SHORTCUT -> ShortcutTile(item, context)
        Kinds.FOLDER -> FolderTile(item, context)
        else -> UnknownItem(item)
    }
}

/** An unknown kind draws nothing but keeps its cell; a long press offers „Entfernen“ only. */
@Composable
private fun UnknownItem(item: HomeItem) {
    Box(Modifier.fillMaxSize().semantics { contentDescription = "Unbekanntes Element ${item.kind}" })
}

/** Semantics of a cell: name and the actions of U35-32 (move, remove, app info for apps). */
@Composable
private fun Modifier.cellSemantics(item: HomeItem, context: ItemContext): Modifier {
    val label = context.label(item)
    val vm = context.vm
    return semantics {
        contentDescription = label
        val actions = mutableListOf(
            CustomAccessibilityAction("Verschieben nach…") {
                vm.moveTarget = item
                true
            },
            CustomAccessibilityAction("Entfernen") {
                vm.removeItem(item.id)
                true
            },
        )
        if (item.isApp) {
            actions += CustomAccessibilityAction("App-Info") {
                vm.startAppInfo(item)
                true
            }
        }
        customActions = actions
    }
}

@Composable
private fun Label(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        modifier = modifier,
        color = LabelColor,
        style = MaterialTheme.typography.labelSmall.copy(shadow = LabelShadow, fontSize = 11.sp, lineHeight = 13.sp),
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
        textAlign = TextAlign.Center,
    )
}

@Composable
internal fun EmptyIcon(size: Dp, modifier: Modifier = Modifier) {
    Box(modifier.size(size).clip(CircleShape).background(Color.White.copy(alpha = 0.25f)))
}

/** The icon of an app, empty until it is loaded off the main thread. */
@Composable
internal fun AppIcon(entry: AppEntry?, context: ItemContext, sizeDp: Int, modifier: Modifier = Modifier) {
    val loader = context.vm.icons
    val px = loader.pixels(sizeDp)
    val key = entry?.let { loader.appKey(it, px) }
    val bitmap by produceState<ImageBitmap?>(initialValue = key?.let { loader.peek(it)?.asImageBitmap() }, key) {
        value = entry?.let { loader.appIcon(it, px)?.asImageBitmap() }
    }
    val image = bitmap
    if (image == null) EmptyIcon(sizeDp.dp, modifier) else Image(image, contentDescription = null, modifier = modifier.size(sizeDp.dp))
}

@Composable
private fun ShortcutIcon(item: HomeItem, info: ShortcutInfo?, context: ItemContext, sizeDp: Int, modifier: Modifier = Modifier) {
    val loader = context.vm.icons
    val px = loader.pixels(sizeDp)
    val key = loader.shortcutKey(item.packageName.orEmpty(), item.attr("shortcutId").orEmpty(), item.userSerial, px)
    val bitmap by produceState<ImageBitmap?>(initialValue = loader.peek(key)?.asImageBitmap(), key, info) {
        value = info?.let { shortcut ->
            loader.load(key, px, persist = false) { context.vm.shortcuts.icon(shortcut, loader.densityDpi) }?.asImageBitmap()
        }
    }
    val image = bitmap
    if (image == null) EmptyIcon(sizeDp.dp, modifier) else Image(image, contentDescription = null, modifier = modifier.size(sizeDp.dp))
}

@Composable
private fun AppTile(item: HomeItem, context: ItemContext) {
    val entry = context.entry(item)
    // Until the apps are read, nothing is greyed out.
    val dim = if (entry == null) context.apps.isNotEmpty() else !entry.available
    Column(
        Modifier.fillMaxSize()
            .alpha(if (dim) 0.5f else 1f)
            .cellSemantics(item, context)
            .clickable { context.vm.launch(item) }
            .padding(horizontal = 2.dp, vertical = 4.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        AppIcon(entry, context, context.iconDp)
        Label(context.label(item), Modifier.padding(top = 2.dp))
    }
}

@Composable
private fun ShortcutTile(item: HomeItem, context: ItemContext) {
    val info = context.shortcut(item)
    Column(
        Modifier.fillMaxSize()
            .cellSemantics(item, context)
            .clickable { context.vm.launch(item) }
            .padding(horizontal = 2.dp, vertical = 4.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        ShortcutIcon(item, info, context, context.iconDp)
        Label(context.label(item), Modifier.padding(top = 2.dp))
    }
}

/** A folder on the home screen: its first four symbols in 2x2 on a rounded plate (U35-23). */
@Composable
private fun FolderTile(item: HomeItem, context: ItemContext) {
    val inner = (context.iconDp / 2 - 4).coerceAtLeast(12)
    Column(
        Modifier.fillMaxSize()
            .cellSemantics(item, context)
            .clickable { context.vm.launch(item) }
            .padding(horizontal = 2.dp, vertical = 4.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Box(
            Modifier.size(context.iconDp.dp).clip(RoundedCornerShape(context.iconDp.dp / 3)).background(Color.White.copy(alpha = 0.3f)).padding(3.dp),
        ) {
            item.children.take(4).forEachIndexed { index, child ->
                Box(Modifier.offset(x = inner.dp * (index % 2), y = inner.dp * (index / 2))) { ChildIcon(child, context, inner) }
            }
        }
        Label(context.label(item), Modifier.padding(top = 2.dp))
    }
}

/** A small symbol of a folder child (app or shortcut). */
@Composable
internal fun ChildIcon(child: HomeItem, context: ItemContext, sizeDp: Int) {
    if (child.isShortcut) ShortcutIcon(child, context.shortcut(child), context, sizeDp) else AppIcon(context.entry(child), context, sizeDp)
}

/** A child of a folder as a tile in the popup (U35-23): symbol and name, tap starts it. */
@Composable
internal fun ChildTile(child: HomeItem, context: ItemContext, modifier: Modifier = Modifier) {
    Column(
        modifier.cellSemantics(child, context).padding(4.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        ChildIcon(child, context, context.iconDp)
        Text(
            context.label(child),
            style = MaterialTheme.typography.labelSmall,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(top = 2.dp),
        )
    }
}

/**
 * One page: every item in its cell. [exclude] is the id of the item being dragged, which leaves its place
 * (it follows the finger instead).
 */
@Composable
fun CellGridPage(
    items: List<HomeItem>,
    metrics: HomeMetrics,
    context: ItemContext,
    exclude: String?,
    modifier: Modifier = Modifier,
) {
    Box(modifier.fillMaxSize()) {
        for (item in items) {
            if (item.id == exclude) continue
            PlacedItem(item, metrics, context)
        }
    }
}

@Composable
internal fun PlacedItem(item: HomeItem, metrics: HomeMetrics, context: ItemContext) {
    val density = androidx.compose.ui.platform.LocalDensity.current
    val cellW = with(density) { metrics.cellW.toDp() }
    val cellH = with(density) { metrics.cellH.toDp() }
    Box(
        Modifier.offset(cellW * item.cell.x, cellH * item.cell.y).size(cellW * item.span.w, cellH * item.span.h),
    ) { ItemContent(item, context) }
}
