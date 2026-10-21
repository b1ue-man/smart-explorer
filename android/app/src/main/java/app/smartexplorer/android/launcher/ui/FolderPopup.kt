package app.smartexplorer.android.launcher.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.background
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.launcher.MenuSource
import app.smartexplorer.android.launcher.model.HomeItem
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.sqrt

/**
 * The open folder (U35-23): a layer with the name on top and the children in a grid of
 * `clamp(ceil(sqrt(n)), 2, columns of the home grid)` columns. A tap on a child starts it; a long press on a
 * child begins a drag onto the home screen, and the child leaves the folder when it is dropped. A tap on the name
 * opens a text field („Fertig“ saves; an empty name becomes „Ordner“). [hidden] makes the layer invisible while
 * a child is dragged, without removing it, so its gesture keeps running.
 */
@Composable
fun FolderPopup(
    folder: HomeItem,
    context: ItemContext,
    host: DragHost,
    maxColumns: Int,
    hidden: Boolean,
    onClose: () -> Unit,
) {
    val vm = context.vm
    var editing by remember(folder.id) { mutableStateOf(vm.renaming) }
    LaunchedEffect(folder.id) { vm.renaming = false }
    var name by remember(folder.id, editing) { mutableStateOf(folder.name) }
    val focus = remember { FocusRequester() }
    LaunchedEffect(editing) { if (editing) focus.requestFocus() }
    fun save() {
        if (editing) {
            vm.renameFolder(folder.id, name)
            editing = false
        }
    }
    val children = context.visibleChildren(folder)
    val columns = ceil(sqrt(max(1, children.size).toDouble())).toInt().coerceIn(2, max(2, maxColumns))

    BoxWithConstraints(
        Modifier.fillMaxSize()
            .alpha(if (hidden) 0f else 1f)
            .background(Color.Black.copy(alpha = 0.45f))
            .clickable(indication = null, interactionSource = null) {
                save()
                onClose()
            },
        contentAlignment = Alignment.Center,
    ) {
        Surface(
            Modifier.padding(24.dp).widthIn(max = 420.dp).clickable(indication = null, interactionSource = null) {},
            shape = MaterialTheme.shapes.extraLarge,
            tonalElevation = 6.dp,
        ) {
            Column(Modifier.padding(16.dp)) {
                if (editing) {
                    TextField(
                        value = name,
                        onValueChange = { name = it },
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                        keyboardActions = KeyboardActions(onDone = { save() }),
                        modifier = Modifier.fillMaxWidth().focusRequester(focus),
                    )
                } else {
                    Text(
                        folder.name,
                        style = MaterialTheme.typography.titleLarge,
                        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable { editing = true },
                    )
                }
                LazyVerticalGrid(
                    columns = GridCells.Fixed(columns),
                    modifier = Modifier.fillMaxWidth().heightIn(max = maxHeight * 0.7f).padding(top = 8.dp),
                ) {
                    items(children, key = { it.id }) { child ->
                        ChildTile(
                            child,
                            context,
                            Modifier
                                .clickable {
                                    vm.launch(child)
                                    onClose()
                                }
                                .dragSource(host, child.id, DragOrigin.Folder, MenuSource.Folder) { child },
                        )
                    }
                }
            }
        }
    }
}
