package app.smartexplorer.android.launcher.se

import androidx.annotation.DrawableRes
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.smartexplorer.android.R
import app.smartexplorer.android.launcher.bridge.StatusCodes
import app.smartexplorer.android.launcher.bridge.Tone
import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.model.Kinds
import app.smartexplorer.android.launcher.ui.ItemContext
import app.smartexplorer.android.launcher.ui.Label
import app.smartexplorer.android.launcher.ui.cellSemantics

private val Green = Color(0xFF4CAF50)
private val Grey = Color(0xFF9E9E9E)
private val Yellow = Color(0xFFFFC107)
private val Red = Color(0xFFF44336)

/** Ring colour of a tone; `null` draws no ring (U40-9: neutral, grey, yellow, green outline, progress, green, red). */
private fun ringColor(tone: Tone): Color = when (tone) {
    Tone.Neutral -> Color.White.copy(alpha = 0.6f)
    Tone.Gray -> Grey
    Tone.Yellow -> Yellow
    Tone.OutlineGreen, Tone.Green -> Green
    Tone.Progress -> Color.White
    Tone.Red -> Red
}

/** The plate behind the symbol: filled for a connected device, translucent otherwise („Erreichbar“ is only an outline). */
private fun plateColor(tone: Tone): Color = when (tone) {
    Tone.Green -> Green.copy(alpha = 0.35f)
    Tone.Red -> Red.copy(alpha = 0.3f)
    Tone.Yellow -> Yellow.copy(alpha = 0.3f)
    else -> Color.White.copy(alpha = 0.25f)
}

/**
 * The frame of every se tile: a symbol on a plate with a ring by [TileView.tone] (a progress ring while connecting), the
 * name, and the state line. [onClick] is the tap; the description for the accessibility service carries name and state.
 */
@Composable
private fun SeTileFrame(item: HomeItem, context: ItemContext, view: TileView, @DrawableRes icon: Int, onClick: () -> Unit) {
    val size = (context.iconDp - 8).coerceAtLeast(32)
    Column(
        Modifier.fillMaxSize()
            .alpha(if (view.gone) 0.55f else 1f)
            .cellSemantics(item, context, view.description)
            .clickable(onClick = onClick)
            .padding(horizontal = 2.dp, vertical = 2.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Box(
            Modifier.size(size.dp).clip(CircleShape).background(plateColor(view.tone)).border(2.dp, ringColor(view.tone), CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            Icon(painterResource(icon), contentDescription = null, tint = Color.White, modifier = Modifier.size((size * 0.55f).dp))
            if (view.progress) CircularProgressIndicator(Modifier.size(size.dp), color = Color.White, strokeWidth = 2.dp)
        }
        Label(view.title, Modifier.padding(top = 2.dp))
        view.subtitle?.let { text ->
            Text(
                text,
                color = Color.White.copy(alpha = 0.85f),
                style = MaterialTheme.typography.labelSmall.copy(fontSize = 9.sp, lineHeight = 10.sp),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                textAlign = TextAlign.Center,
            )
        }
    }
}

/** A device (`seDevice`, U40-9): symbol, name, state line; tap by [SeActions.tapDevice]. */
@Composable
fun SeDeviceTile(item: HomeItem, context: ItemContext) {
    val se = context.vm.se
    val activity = LocalContext.current
    val ui by se.ui.collectAsStateWithLifecycle()
    val view = SeViews.device(item, ui)
    val location = item.attr("target").orEmpty()
    SeTileFrame(item, context, view, R.drawable.ic_device) {
        if (!view.gone) se.open(activity, SeActions.tapDevice(view.code ?: StatusCodes.UNKNOWN, location))
    }
}

/** A room (`seRoom`): folder symbol, name, „n Geräte“; a tap opens the popup with its members. */
@Composable
fun SeRoomTile(item: HomeItem, context: ItemContext) {
    val ui by context.vm.se.ui.collectAsStateWithLifecycle()
    val view = SeViews.room(item, ui)
    SeTileFrame(item, context, view, R.drawable.ic_room) {
        if (!view.gone) context.vm.openRoomId = item.attr("target")
    }
}

/** A vault site (`seVault`): lock symbol, label, state. Nothing of the content is known to the launcher (S5-V5). */
@Composable
fun SeVaultTile(item: HomeItem, context: ItemContext) {
    val se = context.vm.se
    val activity = LocalContext.current
    val ui by se.ui.collectAsStateWithLifecycle()
    val view = SeViews.vault(item, ui)
    val site = item.attr("target").orEmpty().removePrefix("vault://").trimEnd('/')
    SeTileFrame(item, context, view, R.drawable.ic_lock) {
        if (!view.gone) se.open(activity, SeActions.tapVault(site))
    }
}

/** „Alle Bildschirme“ (`seScreens`): screen symbol, no state, no target. */
@Composable
fun SeScreensTile(item: HomeItem, context: ItemContext) {
    val se = context.vm.se
    val activity = LocalContext.current
    SeTileFrame(item, context, SeViews.screens(), R.drawable.ic_screens) { se.open(activity, SeActions.screenGrid()) }
}

/** „Gerät hinzufügen“ and „Raum beitreten“ of an empty device list: both open the Share tab (U40-9). */
@Composable
fun SeAddTile(item: HomeItem, context: ItemContext) {
    val se = context.vm.se
    val activity = LocalContext.current
    val title = SeViews.title(item, se.ui.value)
    val icon = if (item.kind == Kinds.SE_ADD_DEVICE) R.drawable.ic_add else R.drawable.ic_room
    val view = TileView(title, null, Tone.Neutral, false, false, title)
    SeTileFrame(item, context, view, icon) { se.open(activity, SeActions.SHARE_TAB) }
}

/** One member of a room in the popup: the same symbol, name and state line as a device tile (U40-9). */
@Composable
fun SeMemberTile(member: app.smartexplorer.android.launcher.bridge.MemberEntry, ui: SeUiState, modifier: Modifier = Modifier) {
    val view = SeViews.member(member, ui)
    Column(modifier.padding(4.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Box(
            Modifier.size(48.dp).clip(CircleShape).background(plateColor(view.tone).copy(alpha = 0.5f)).border(2.dp, ringColor(view.tone), CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            Icon(painterResource(R.drawable.ic_device), contentDescription = null, modifier = Modifier.size(26.dp))
            if (view.progress) CircularProgressIndicator(Modifier.size(48.dp), strokeWidth = 2.dp)
        }
        Text(view.title, style = MaterialTheme.typography.labelMedium, maxLines = 1, overflow = TextOverflow.Ellipsis, textAlign = TextAlign.Center, modifier = Modifier.padding(top = 2.dp))
        view.subtitle?.let {
            Text(it, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2, overflow = TextOverflow.Ellipsis, textAlign = TextAlign.Center)
        }
    }
}
