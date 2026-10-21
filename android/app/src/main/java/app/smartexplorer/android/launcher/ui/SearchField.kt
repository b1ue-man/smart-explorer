package app.smartexplorer.android.launcher.ui

import android.app.SearchManager
import android.content.Context
import android.content.Intent
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.launcher.MenuSource
import app.smartexplorer.android.launcher.apps.AppEntry
import app.smartexplorer.android.launcher.search.LauncherSearch

/** The search field at the top of the app list (U35-25). */
@Composable
fun SearchField(query: String, onQuery: (String) -> Unit, modifier: Modifier = Modifier) {
    val keyboard = LocalSoftwareKeyboardController.current
    TextField(
        value = query,
        onValueChange = onQuery,
        modifier = modifier.fillMaxWidth().heightIn(min = 56.dp),
        singleLine = true,
        placeholder = { Text("Apps suchen") },
        leadingIcon = { Icon(Icons.Default.Search, contentDescription = null) },
        trailingIcon = {
            if (query.isNotEmpty()) {
                IconButton(onClick = { onQuery("") }) { Icon(Icons.Default.Close, contentDescription = "Suche leeren") }
            }
        },
        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
        keyboardActions = KeyboardActions(onSearch = { keyboard?.hide() }),
    )
}

private fun webSearch(query: String): Intent = Intent(Intent.ACTION_WEB_SEARCH).putExtra(SearchManager.QUERY, query)

private fun canWebSearch(context: Context, query: String): Boolean = webSearch(query).resolveActivity(context.packageManager) != null

/**
 * The hits for [query] (U35-25): „Apps“; „Verknüpfungen“ of the first matching app, only with the shortcut host
 * permission; one section per additional source (U40 registers „Geräte und Räume“); „Im Web suchen“ last, when
 * an app can answer it. The match runs over the app list in memory, without an index.
 */
@Composable
fun SearchResults(
    query: String,
    apps: List<AppEntry>,
    context: ItemContext,
    host: DragHost,
    modifier: Modifier = Modifier,
) {
    val activity = LocalContext.current
    val vm = context.vm
    val matches = remember(query, apps) { LauncherSearch.filter(apps, query, { it.label }) }
    val shortcuts = remember(matches) {
        matches.firstOrNull()?.let { first -> vm.shortcuts.forPackage(first.packageName, first.user) }.orEmpty()
    }
    val extra = remember(query) { LauncherSearch.additional(query).groupBy { it.source } }
    val web = remember(query) { canWebSearch(activity, query) }
    LazyColumn(modifier.fillMaxSize()) {
        if (matches.isNotEmpty()) {
            item(key = "h-apps") { Heading("Apps") }
            items(matches, key = { "a-" + it.key }) { entry ->
                AppRow(entry, context, host, MenuSource.Search)
            }
        }
        if (shortcuts.isNotEmpty()) {
            item(key = "h-shortcuts") { Heading("Verknüpfungen") }
            items(shortcuts, key = { "s-" + it.id }) { info ->
                TextRow(info.shortLabel?.toString() ?: info.id) { vm.shortcuts.start(info) }
            }
        }
        extra.forEach { (source, hits) ->
            item(key = "h-$source") { Heading(source) }
            items(hits, key = { "x-$source-${it.title}" }) { hit ->
                TextRow(hit.title, hit.subtitle) { hit.onClick() }
            }
        }
        if (web) {
            item(key = "web") {
                TextRow("Im Web suchen: $query") {
                    try {
                        activity.startActivity(webSearch(query).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                    } catch (_: Exception) {
                        vm.showToast("Websuche ist nicht verfügbar.")
                    }
                }
            }
        }
    }
}

@Composable
private fun Heading(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 4.dp),
    )
}

@Composable
private fun TextRow(title: String, subtitle: String? = null, onClick: () -> Unit) {
    Column(
        Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(onClick = onClick).padding(horizontal = 16.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.Center,
    ) {
        Text(title, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
        if (subtitle != null) {
            Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

/** An app as a row: icon and name, tap starts it, a long press opens the menu or drags it to the home screen. */
@Composable
private fun AppRow(entry: AppEntry, context: ItemContext, host: DragHost, source: MenuSource) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp)
            .clickable { context.vm.launchEntry(entry) }
            .dragSource(host, entry.key, DragOrigin.Drawer, source) { context.vm.appItem(entry) }
            .padding(horizontal = 16.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        WithDot(context.hasDot(entry)) { AppIcon(entry, context, 40) }
        Text(
            entry.label,
            style = MaterialTheme.typography.bodyLarge,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            color = if (entry.available) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.5f),
        )
    }
}
