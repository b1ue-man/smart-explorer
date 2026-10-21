package app.smartexplorer.android.launcher.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.requiredHeight
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.PagerState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.launcher.model.HomeItem

/**
 * The pages (U35-21). While an item is dragged or the settings sheet is open, one more empty page, „Neue
 * Seite“, stands to the right of the last one; dropping there creates it. [scrollEnabled] is false during a drag.
 */
@Composable
fun HomePager(
    state: PagerState,
    pages: List<List<HomeItem>>,
    showNewPage: Boolean,
    metrics: HomeMetrics,
    context: ItemContext,
    exclude: String?,
    scrollEnabled: Boolean,
    modifier: Modifier = Modifier,
) {
    HorizontalPager(state = state, userScrollEnabled = scrollEnabled, modifier = modifier) { index ->
        if (index < pages.size) {
            CellGridPage(pages[index], metrics, context, exclude)
        } else if (showNewPage) {
            Box(Modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
                Box(
                    Modifier.fillMaxSize().clip(RoundedCornerShape(24.dp)).background(Color.White.copy(alpha = 0.18f))
                        .semantics { contentDescription = "Neue Seite" },
                    contentAlignment = Alignment.Center,
                ) { Text("Neue Seite", color = Color.White, style = MaterialTheme.typography.titleMedium) }
            }
        }
    }
}

/**
 * The row below the pages (U35-21): the dots, and „Alle Apps“ with a touch target of 48 dp. The dots read
 * „Seite 2 von 4“ to the accessibility service (U35-32).
 */
@Composable
fun PageIndicator(current: Int, count: Int, onAllApps: () -> Unit, modifier: Modifier = Modifier) {
    Box(modifier.fillMaxWidth()) {
        Row(
            Modifier.align(Alignment.Center).semantics { contentDescription = "Seite ${current + 1} von $count" },
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (count > 1) {
                repeat(count) { index ->
                    Box(
                        Modifier.size(if (index == current) 8.dp else 6.dp).clip(CircleShape)
                            .background(Color.White.copy(alpha = if (index == current) 0.95f else 0.5f)),
                    )
                }
            }
        }
        Box(
            Modifier.align(Alignment.CenterEnd)
                .requiredHeight(48.dp)
                .clip(RoundedCornerShape(24.dp))
                .clickable(onClickLabel = "Alle Apps öffnen", role = Role.Button, onClick = onAllApps)
                .padding(horizontal = 16.dp)
                .semantics { contentDescription = "Alle Apps" },
            contentAlignment = Alignment.Center,
        ) { Text("Alle Apps", color = Color.White, style = MaterialTheme.typography.labelLarge) }
    }
}
