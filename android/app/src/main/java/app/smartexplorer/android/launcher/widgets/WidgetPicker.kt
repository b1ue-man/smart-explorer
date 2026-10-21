package app.smartexplorer.android.launcher.widgets

import android.appwidget.AppWidgetProviderInfo
import android.graphics.drawable.Drawable
import android.os.UserHandle
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.launcher.LauncherViewModel
import app.smartexplorer.android.launcher.model.Span
import app.smartexplorer.android.launcher.search.LauncherSearch
import java.text.Collator
import java.util.Locale
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** One widget of a provider in the picker: its name and the span it would start with ([Span] as „B × H“). */
data class PickerEntry(val info: AppWidgetProviderInfo, val label: String, val span: Span) {
    val sizeText: String get() = "${span.w} × ${span.h}"
}

/** The widgets of one app in one profile (U38-10). [icon] is the app's icon, drawn off the main thread. */
data class PickerGroup(
    val key: String,
    val label: String,
    val user: UserHandle,
    val userSerial: Long,
    val icon: Drawable?,
    val entries: List<PickerEntry>,
)

/**
 * True when the words of the query all match the app name together with the widget name (U38-10), with the
 * normalisation of the app search (U35-25); an empty query matches everything.
 */
fun pickerMatches(tokens: List<String>, appLabel: String, widgetLabel: String): Boolean =
    tokens.isEmpty() || LauncherSearch.rank(tokens, "$appLabel $widgetLabel") != null

/** The groups that have at least one widget matching [query]; an entry matches by its own name or its app's. */
fun filterGroups(groups: List<PickerGroup>, query: String): List<PickerGroup> {
    val tokens = LauncherSearch.tokens(query)
    if (tokens.isEmpty()) return groups
    return groups.mapNotNull { group ->
        val entries = group.entries.filter { pickerMatches(tokens, group.label, it.label) }
        if (entries.isEmpty()) null else group.copy(entries = entries)
    }
}

/**
 * Reads the providers of every profile of `AppCatalog` (U38-10), the owner's apps first, and groups them by app. The
 * start span of each entry comes from [WidgetSizes.initialSpan] with the current grid.
 */
suspend fun loadPickerGroups(vm: LauncherViewModel): List<PickerGroup> = withContext(Dispatchers.Default) {
    val context = vm.getApplication<android.app.Application>()
    val pm = context.packageManager
    val density = context.resources.displayMetrics.density
    val grids = vm.sizeGrids
    val collator = Collator.getInstance(Locale.getDefault()).apply { strength = Collator.PRIMARY }
    val result = ArrayList<PickerGroup>()
    for (profile in vm.catalog.profilesForPicker()) {
        val groups = vm.widgets.providersOf(profile.user).groupBy { it.provider.packageName }.mapNotNull { (pkg, infos) ->
            val application = infos.first().providerInfo.applicationInfo
            val appLabel = pm.getUserBadgedLabel(application.loadLabel(pm), profile.user).toString()
            val entries = infos.map { info ->
                val span = WidgetSizes.initialSpan(
                    targetCellWidth = if (android.os.Build.VERSION.SDK_INT >= 31) info.targetCellWidth else 0,
                    targetCellHeight = if (android.os.Build.VERSION.SDK_INT >= 31) info.targetCellHeight else 0,
                    minWidthPx = info.minWidth,
                    minHeightPx = info.minHeight,
                    geometry = grids.current,
                    density = density,
                )
                PickerEntry(info, info.loadLabel(pm).ifEmpty { pkg }, span)
            }.sortedWith { a, b -> collator.compare(a.label, b.label) }
            PickerGroup("$pkg#${profile.userSerial}", appLabel, profile.user, profile.userSerial, runCatching { application.loadIcon(pm) }.getOrNull(), entries)
        }.sortedWith { a, b -> collator.compare(a.label, b.label) }
        result += groups
    }
    result
}

/**
 * The sheet „Widgets“ at the bottom edge (U38-10): search field „Widgets suchen“, one expandable group per app with
 * the number of its widgets, and per widget the preview, the name, the size in cells and the button „Hinzufügen“.
 * There is no dragging out of the sheet; the button is also the alternative for the accessibility service.
 */
@Composable
fun WidgetPicker(vm: LauncherViewModel, onClose: () -> Unit, modifier: Modifier = Modifier) {
    val groups by produceState<List<PickerGroup>?>(null, vm.sizeGrids.current) { value = loadPickerGroups(vm) }
    var query by remember { mutableStateOf("") }
    val expanded = remember { mutableStateMapOf<String, Boolean>() }
    val shown = remember(groups, query) { groups?.let { filterGroups(it, query) } }
    val maxHeight = LocalConfiguration.current.screenHeightDp.dp * 0.8f
    Box(
        Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.45f)).clickable(indication = null, interactionSource = null, onClick = onClose),
        contentAlignment = Alignment.BottomCenter,
    ) {
        Surface(
            modifier.fillMaxWidth().heightIn(max = maxHeight).clickable(indication = null, interactionSource = null) {},
            shape = MaterialTheme.shapes.extraLarge,
            tonalElevation = 6.dp,
            shadowElevation = 8.dp,
        ) {
            Column(Modifier.padding(16.dp)) {
                Text("Widgets", style = MaterialTheme.typography.titleMedium)
                TextField(
                    value = query,
                    onValueChange = { query = it },
                    modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                    singleLine = true,
                    placeholder = { Text("Widgets suchen") },
                    leadingIcon = { Icon(Icons.Default.Search, contentDescription = null) },
                    trailingIcon = {
                        if (query.isNotEmpty()) IconButton(onClick = { query = "" }) { Icon(Icons.Default.Close, contentDescription = "Suche leeren") }
                    },
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                )
                val list = shown
                when {
                    list == null -> Unit
                    list.isEmpty() -> Text("Keine Widgets verfügbar.", Modifier.padding(vertical = 24.dp), style = MaterialTheme.typography.bodyLarge)
                    else -> LazyColumn(Modifier.fillMaxWidth().padding(top = 8.dp)) {
                        items(list, key = { it.key }) { group ->
                            val open = query.isNotBlank() || expanded[group.key] == true
                            GroupHeader(vm, group, open) { expanded[group.key] = !open }
                            if (open) group.entries.forEach { entry -> EntryRow(entry, group.user) { vm.addWidget(entry.info, entry.span) } }
                        }
                    }
                }
                Row(Modifier.fillMaxWidth().padding(top = 8.dp), horizontalArrangement = Arrangement.End) {
                    TextButton(onClick = onClose) { Text("Schließen") }
                }
            }
        }
    }
}

@Composable
private fun GroupHeader(vm: LauncherViewModel, group: PickerGroup, open: Boolean, onToggle: () -> Unit) {
    val px = vm.icons.pixels(40)
    val icon by produceState<ImageBitmap?>(null, group.key) {
        value = group.icon?.let { drawable -> vm.icons.load("wp~${group.key}~$px", px, persist = false) { drawable }?.asImageBitmap() }
    }
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Button, onClick = onToggle)
            .semantics { stateDescription = if (open) "aufgeklappt" else "zugeklappt" },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        val image = icon
        if (image != null) Image(image, contentDescription = null, modifier = Modifier.size(40.dp)) else Box(Modifier.size(40.dp))
        Text("${group.label} (${group.entries.size})", style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
private fun EntryRow(entry: PickerEntry, user: UserHandle, onAdd: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(start = 56.dp, top = 4.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Box(Modifier.size(width = 96.dp, height = 72.dp), contentAlignment = Alignment.Center) { WidgetPreview(entry.info, user, Modifier.fillMaxSize()) }
        Column(Modifier.weight(1f)) {
            Text(entry.label, style = MaterialTheme.typography.bodyLarge, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Text(entry.sizeText, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Button(onClick = onAdd) { Text("Hinzufügen") }
    }
}
