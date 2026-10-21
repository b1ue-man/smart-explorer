package app.smartexplorer.android.launcher.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.nestedscroll.NestedScrollConnection
import androidx.compose.ui.input.nestedscroll.NestedScrollSource
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Velocity
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.launcher.MenuSource
import app.smartexplorer.android.launcher.apps.AppEntry
import app.smartexplorer.android.launcher.apps.ProfileInfo
import app.smartexplorer.android.launcher.apps.ProfileKind
import app.smartexplorer.android.launcher.model.GridGeometry

/** Title of the work tab; the private space is not a tab but a section below „Persönlich“ (U38-14). */
private const val WORK_TAB = "Arbeit"

/** A tab of the app list: its title and which apps it shows (U35-31). */
data class DrawerTab(val title: String, val accepts: (AppEntry) -> Boolean)

/** „Persönlich“ always, „Arbeit“ only when a work profile exists (U35-24). */
fun profileTabs(profiles: List<ProfileInfo>): List<DrawerTab> {
    val tabs = ArrayList<DrawerTab>()
    tabs += DrawerTab("Persönlich") { it.kind == ProfileKind.Personal || it.kind == ProfileKind.Other }
    if (profiles.any { it.isWork }) tabs += DrawerTab(WORK_TAB) { it.kind == ProfileKind.Work }
    return tabs
}

/**
 * The app list (U35-24): full screen, the search field at the top, tabs per profile, a grid of all apps. A tap
 * starts an app; a long press opens the menu or, when the finger moves, drags the app to the home screen (the
 * list then closes). It closes with „Zurück“, by pulling down from the top of the list, and when an app starts.
 * [hidden] makes the layer invisible during such a drag without removing it, so the gesture goes on.
 */
@Composable
fun AppDrawer(
    apps: List<AppEntry>,
    profiles: List<ProfileInfo>,
    context: ItemContext,
    host: DragHost,
    iconDp: Int,
    insets: PaddingValues,
    hidden: Boolean,
    showPrivate: Boolean,
    onClose: () -> Unit,
) {
    val vm = context.vm
    val tabs = remember(profiles) { profileTabs(profiles) }
    var tab by remember { mutableIntStateOf(0) }
    val selected = tab.coerceIn(0, tabs.size - 1)
    var privateOpen by rememberSaveable { mutableStateOf(true) }
    val workProfile = profiles.firstOrNull { it.isWork }
    val privateProfile = profiles.firstOrNull { it.isPrivate }?.takeIf { showPrivate }
    val density = LocalDensity.current
    val closeDistance = with(density) { 96.dp.toPx() }
    val connection = remember(closeDistance) { PullDownToClose(closeDistance, onClose) }

    Surface(
        Modifier.fillMaxSize().alpha(if (hidden) 0f else 1f),
        color = MaterialTheme.colorScheme.surface.copy(alpha = 0.94f),
    ) {
        Column(Modifier.fillMaxSize().padding(insets).nestedScroll(connection)) {
            SearchField(vm.query, { vm.query = it })
            if (vm.query.isNotBlank()) {
                SearchResults(vm.query, apps, context, host)
            } else {
                if (tabs.size > 1) {
                    Row(Modifier.fillMaxWidth()) {
                        tabs.forEachIndexed { index, item ->
                            TabHeader(item.title, index == selected, Modifier.weight(1f)) { tab = index }
                        }
                    }
                }
                val shown = remember(apps, selected, tabs) { apps.filter(tabs[selected].accepts) }
                // „Arbeit“: the switch and, while paused, the banner (U38-13).
                if (workProfile != null && tabs[selected].title == WORK_TAB) WorkProfileRows(workProfile, context)
                LazyVerticalGrid(
                    columns = GridCells.Adaptive(GridGeometry.cellMinWidthDp(iconDp).dp),
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(horizontal = 8.dp, vertical = 8.dp),
                ) {
                    items(shown, key = { it.key }) { entry -> DrawerCell(entry, context, host) }
                    // „Persönlich“ ends with the section of the private space, when the person lets it show (U38-14).
                    if (privateProfile != null && selected == 0) {
                        privateSpaceSection(privateProfile, apps.filter { it.kind == ProfileKind.Private }, privateOpen, { privateOpen = !privateOpen }, context, host)
                    }
                }
            }
        }
    }
}

/** A profile tab: the title with a line under the selected one; a touch target of at least 48 dp. */
@Composable
private fun TabHeader(title: String, selected: Boolean, modifier: Modifier, onClick: () -> Unit) {
    val color = if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant
    Column(
        modifier.heightIn(min = 48.dp).clickable(role = Role.Tab, onClick = onClick).semantics { this.selected = selected },
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Text(title, color = color, style = MaterialTheme.typography.titleSmall)
        Box(Modifier.padding(top = 6.dp).fillMaxWidth().height(2.dp).background(if (selected) color else androidx.compose.ui.graphics.Color.Transparent))
    }
}

/**
 * The top of the tab „Arbeit“ (U38-13): the switch „Arbeitsprofil pausieren“ (on while the profile is paused) and, while it
 * is paused, the banner „Arbeits-Apps sind pausiert.“ with „Fortsetzen“. Pausing works only for the default home app (R60).
 */
@Composable
private fun WorkProfileRows(profile: ProfileInfo, context: ItemContext) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
        Row(
            Modifier.fillMaxWidth().heightIn(min = 48.dp),
            horizontalArrangement = Arrangement.spacedBy(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("Arbeitsprofil pausieren", Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
            Switch(checked = profile.isQuiet, onCheckedChange = { context.vm.setQuiet(profile, it) })
        }
        if (profile.isQuiet) {
            Row(
                Modifier.fillMaxWidth().heightIn(min = 48.dp),
                horizontalArrangement = Arrangement.spacedBy(16.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("Arbeits-Apps sind pausiert.", Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium)
                TextButton(onClick = { context.vm.setQuiet(profile, false) }) { Text("Fortsetzen") }
            }
        }
    }
}

@Composable
internal fun DrawerCell(entry: AppEntry, context: ItemContext, host: DragHost) {
    Column(
        Modifier.fillMaxWidth()
            .alpha(if (entry.available) 1f else 0.5f)
            .clickable { context.vm.launchEntry(entry) }
            .dragSource(host, entry.key, DragOrigin.Drawer, MenuSource.Drawer) { context.vm.appItem(entry) }
            .padding(vertical = 8.dp, horizontal = 2.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        WithDot(context.hasDot(entry)) { AppIcon(entry, context, context.iconDp) }
        Text(
            entry.label,
            style = MaterialTheme.typography.labelMedium,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(top = 4.dp),
        )
    }
}

/**
 * Closes the list when the person pulls down further than [distancePx] while the grid is already at its top,
 * or flings down hard: the part of a downward scroll the grid could not use arrives in `onPostScroll`.
 */
private class PullDownToClose(private val distancePx: Float, private val close: () -> Unit) : NestedScrollConnection {
    private var pulled = 0f

    override fun onPostScroll(consumed: Offset, available: Offset, source: NestedScrollSource): Offset {
        if (source == NestedScrollSource.UserInput) {
            pulled = if (available.y > 0f) pulled + available.y else 0f
        }
        return Offset.Zero
    }

    override suspend fun onPostFling(consumed: Velocity, available: Velocity): Velocity {
        val fling = available.y > FLING_CLOSE_SPEED
        val far = pulled >= distancePx
        pulled = 0f
        if (far || fling) close()
        return Velocity.Zero
    }

    private companion object {
        /** Pixels per second of a fling that closes the list from the top. */
        const val FLING_CLOSE_SPEED = 2000f
    }
}
