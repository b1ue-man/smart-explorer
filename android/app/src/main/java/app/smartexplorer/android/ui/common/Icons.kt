package app.smartexplorer.android.ui.common

import androidx.annotation.DrawableRes
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalContentColor
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import app.smartexplorer.android.R
import app.smartexplorer.android.core.Root

/** Icon from the app's vector symbols (`res/drawable/ic_*.xml`, Material Symbols). */
@Composable
fun SeIcon(
    @DrawableRes id: Int,
    contentDescription: String?,
    modifier: Modifier = Modifier,
    tint: Color = LocalContentColor.current,
) {
    Icon(painterResource(id), contentDescription, modifier, tint)
}

/** Symbol for `Entry.kind` (api.md §2). */
@DrawableRes
fun kindIcon(kind: String): Int = when (kind) {
    "dir" -> R.drawable.ic_folder
    "image" -> R.drawable.ic_image
    "video" -> R.drawable.ic_video
    "audio" -> R.drawable.ic_audio
    "text" -> R.drawable.ic_text
    "archive" -> R.drawable.ic_archive
    "document" -> R.drawable.ic_document
    "apk" -> R.drawable.ic_apk
    else -> R.drawable.ic_file
}

/** Symbol for a place in the sidebar (`Root.kind`, api.md §2). */
@DrawableRes
fun rootIcon(root: Root): Int = when (root.kind) {
    "storage" -> when {
        !root.removable -> R.drawable.ic_storage
        root.label.contains("USB", ignoreCase = true) -> R.drawable.ic_usb
        else -> R.drawable.ic_sd_card
    }
    "favorite" -> R.drawable.ic_star
    "recent" -> R.drawable.ic_history
    "connection" -> R.drawable.ic_cloud
    "gdrive" -> R.drawable.ic_drive
    "device" -> R.drawable.ic_device
    "room" -> R.drawable.ic_room
    "trash" -> R.drawable.ic_trash
    else -> R.drawable.ic_folder
}
