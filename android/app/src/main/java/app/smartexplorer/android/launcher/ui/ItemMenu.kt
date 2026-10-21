package app.smartexplorer.android.launcher.ui

import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.ShortcutInfo
import android.net.Uri
import android.os.Process
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.launcher.LauncherViewModel
import app.smartexplorer.android.launcher.MenuSource
import app.smartexplorer.android.launcher.MenuTarget
import app.smartexplorer.android.launcher.apps.AppEntry
import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.model.Kinds
import app.smartexplorer.android.launcher.se.seMenuRows
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlin.math.roundToInt

/** One row of the menu. [shortcut] marks a row that can be dragged to the home screen (U35-26). */
data class MenuRow(val label: String, val shortcut: ShortcutInfo? = null, val action: () -> Unit)

/** What the rows of the menu need: the view model, where the press happened and a context to start activities from. */
class MenuContext(val vm: LauncherViewModel, val source: MenuSource, val activity: Context)

/** An app can be uninstalled unless it is a system app without an update (U35-22). */
fun isUninstallable(entry: AppEntry?): Boolean {
    val flags = entry?.info?.applicationInfo?.flags ?: return false
    return (flags and ApplicationInfo.FLAG_SYSTEM) == 0 || (flags and ApplicationInfo.FLAG_UPDATED_SYSTEM_APP) != 0
}

/**
 * `ACTION_DELETE` for the package, started from the visible activity; the result is not evaluated, the item
 * leaves the home screen by `onPackageRemoved` (U35-28).
 */
fun uninstall(context: Context, entry: AppEntry) {
    val intent = Intent(Intent.ACTION_DELETE, Uri.parse("package:${entry.packageName}"))
    if (entry.user != Process.myUserHandle()) intent.putExtra(Intent.EXTRA_USER, entry.user)
    try {
        context.startActivity(intent)
    } catch (_: Exception) {
        // The system uninstaller is missing; nothing to uninstall with.
    }
}

/**
 * Rows of the menu for [item] by its kind (U35-27, U35-31): U38 added „Größe ändern“ and „Neu einrichten“ for a
 * widget in the same `when`, the se kinds of U40 (`launcher/se/SeMenu.kt`) have their own branch.
 */
fun menuRows(item: HomeItem, context: MenuContext): List<MenuRow> {
    val vm = context.vm
    return when (item.kind) {
        Kinds.APP -> appRows(item, context)
        Kinds.SHORTCUT -> listOf(MenuRow("Entfernen") { vm.removeItem(item.id) })
        Kinds.FOLDER -> listOf(
            MenuRow("Umbenennen") {
                vm.renaming = true
                vm.openFolder(item.id)
            },
            MenuRow("Ordner auflösen") { vm.dissolveFolder(item.id) },
            MenuRow("Entfernen") { vm.dissolveFolder(item.id) },
        )
        Kinds.WIDGET -> widgetRows(item, context)
        Kinds.SE_DEVICE, Kinds.SE_ROOM, Kinds.SE_VAULT, Kinds.SE_SCREENS -> seMenuRows(item, context)
        else -> listOf(MenuRow("Entfernen") { vm.removeItem(item.id) })
    }
}

/** „Größe ändern“ where the provider allows it, „Neu einrichten“ where it is reconfigurable, always „Entfernen“ (U38-8, U38-9). */
private fun widgetRows(item: HomeItem, context: MenuContext): List<MenuRow> {
    val vm = context.vm
    val info = item.widgetId?.let { vm.widgets.manager.getAppWidgetInfo(it) }
    val rows = ArrayList<MenuRow>()
    if (info != null && vm.canResize(item)) rows += MenuRow("Größe ändern") { vm.startResize(item) }
    if (info != null && vm.widgets.canReconfigure(info)) rows += MenuRow("Neu einrichten") { vm.setUpWidget(item) }
    rows += MenuRow("Entfernen") { vm.removeItem(item.id) }
    return rows
}

private fun appRows(item: HomeItem, context: MenuContext): List<MenuRow> {
    val vm = context.vm
    val entry = vm.entryFor(item)
    val rows = ArrayList<MenuRow>()
    if (entry != null) {
        vm.shortcuts.forActivity(entry.component, entry.user).forEach { info ->
            rows += MenuRow(info.shortLabel?.toString() ?: info.id, info) { vm.shortcuts.start(info) }
        }
    }
    rows += MenuRow("App-Info") { vm.startAppInfo(item) }
    if (entry != null && isUninstallable(entry)) rows += MenuRow("Deinstallieren") { uninstall(context.activity, entry) }
    if (context.source == MenuSource.Home || context.source == MenuSource.Folder) {
        rows += MenuRow("Entfernen") { vm.removeItem(item.id) }
    } else if (entry != null) {
        rows += MenuRow("Zum Startbildschirm") { vm.addToHome(entry) }
    }
    return rows
}

/** The shortcut as a home screen item, to be dragged out of the menu (U35-26). */
fun shortcutItem(info: ShortcutInfo, userSerial: Long, component: String): HomeItem = HomeItem(
    id = LauncherViewModel.TRANSIENT_PREFIX + "sc:" + info.`package` + ":" + info.id,
    kind = Kinds.SHORTCUT,
    attrs = JsonObject(
        mapOf(
            "component" to JsonPrimitive(info.activity?.flattenToString() ?: component),
            "shortcutId" to JsonPrimitive(info.id),
            "user" to JsonPrimitive(userSerial),
        ),
    ),
)

/**
 * The menu as a layer inside the home screen, not a popup window: its shortcut rows can be dragged to the home
 * screen, which needs the same pointer tree (U35-26). A tap on the dim background closes it.
 */
@Composable
fun ItemMenu(
    target: MenuTarget,
    context: MenuContext,
    host: DragHost,
    screenWidthPx: Float,
    screenHeightPx: Float,
    onClose: () -> Unit,
) {
    val rows = remember(target) { menuRows(target.item, context) }
    val density = LocalDensity.current
    val menuWidthPx = with(density) { MENU_WIDTH_DP.dp.toPx() }
    val menuHeightPx = with(density) { (rows.size * ROW_HEIGHT_DP + 16).dp.toPx() }
    val margin = with(density) { 8.dp.toPx() }
    val x = (target.anchor.x - menuWidthPx / 2).coerceIn(margin, (screenWidthPx - menuWidthPx - margin).coerceAtLeast(margin))
    val below = target.anchor.y + menuHeightPx + with(density) { 56.dp.toPx() } < screenHeightPx
    val y = if (below) target.anchor.y + with(density) { 40.dp.toPx() } else (target.anchor.y - menuHeightPx - with(density) { 40.dp.toPx() }).coerceAtLeast(margin)
    Box(Modifier.fillMaxSize().clickable(indication = null, interactionSource = null, onClick = onClose)) {
        Surface(
            Modifier.offset { IntOffset(x.roundToInt(), y.roundToInt()) }.width(MENU_WIDTH_DP.dp),
            shape = MaterialTheme.shapes.medium,
            tonalElevation = 6.dp,
            shadowElevation = 8.dp,
        ) {
            Column(Modifier.padding(vertical = 8.dp)) {
                rows.forEach { row -> MenuRowView(row, target, context, host, onClose) }
            }
        }
    }
}

@Composable
private fun MenuRowView(row: MenuRow, target: MenuTarget, context: MenuContext, host: DragHost, onClose: () -> Unit) {
    val info = row.shortcut
    val entry = context.vm.entryFor(target.item)
    var modifier = Modifier.fillMaxWidth().heightIn(min = ROW_HEIGHT_DP.dp)
    if (info != null && entry != null) {
        // Dragging a shortcut row puts it on the home screen; a tap starts it.
        modifier = modifier.dragSource(host, info.id, DragOrigin.Menu, MenuSource.Home) {
            shortcutItem(info, entry.userSerial, entry.componentString)
        }
    }
    Box(
        modifier
            .clickable(role = Role.Button) {
                row.action()
                onClose()
            }
            .padding(horizontal = 16.dp),
        contentAlignment = Alignment.CenterStart,
    ) {
        Text(row.label, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

private const val MENU_WIDTH_DP = 240
private const val ROW_HEIGHT_DP = 48
