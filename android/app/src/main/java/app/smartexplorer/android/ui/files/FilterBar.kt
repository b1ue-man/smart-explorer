package app.smartexplorer.android.ui.files

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.FilterChip
import androidx.compose.material3.IconButton
import androidx.compose.material3.InputChip
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.ui.common.SeIcon

/**
 * Filter row under the path bar (spec F5): search field (focused when opened; several terms with
 * `;`), "Rekursiv" switch, [Filter] for the sheet, and the active criteria as removable chips.
 */
@Composable
internal fun FilterBar(tab: BrowserTab, onOpenSheet: () -> Unit) {
    val filter = tab.filter
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    val chips = criterionChips(filter.criteria)
    Column(Modifier.fillMaxWidth().padding(horizontal = 8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            val error = filter.error
            OutlinedTextField(
                value = filter.text,
                onValueChange = {
                    filter.text = it
                    tab.filterChanged()
                },
                placeholder = { Text("Name…") },
                singleLine = true,
                isError = error != null,
                supportingText = if (error != null) {
                    { Text(error) }
                } else {
                    null
                },
                trailingIcon = if (filter.text.isNotEmpty()) {
                    {
                        IconButton(onClick = {
                            filter.text = ""
                            tab.filterChanged(immediate = true)
                        }) { SeIcon(R.drawable.ic_close, contentDescription = "Name löschen") }
                    }
                } else {
                    null
                },
                modifier = Modifier.weight(1f).focusRequester(focus),
            )
            FilterChip(
                selected = filter.recursive,
                onClick = { tab.setRecursive(!filter.recursive) },
                label = { Text("Rekursiv") },
            )
            IconButton(onClick = onOpenSheet) {
                if (chips.isEmpty()) {
                    SeIcon(R.drawable.ic_filter, contentDescription = "Filter")
                } else {
                    BadgedBox(badge = { Badge { Text(chips.size.toString()) } }) {
                        SeIcon(R.drawable.ic_filter, contentDescription = "Filter, ${chips.size} aktiv")
                    }
                }
            }
        }
        if (chips.isNotEmpty()) {
            Row(
                modifier = Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                chips.forEach { chip ->
                    InputChip(
                        selected = false,
                        onClick = {
                            filter.criteria = chip.clear(filter.criteria)
                            tab.filterChanged(immediate = true)
                        },
                        label = { Text(chip.label) },
                        trailingIcon = { SeIcon(R.drawable.ic_close, contentDescription = "Entfernen", modifier = Modifier.size(18.dp)) },
                    )
                }
            }
        }
    }
}
