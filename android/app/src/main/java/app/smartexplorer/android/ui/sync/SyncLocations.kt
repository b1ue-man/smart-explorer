package app.smartexplorer.android.ui.sync

import androidx.annotation.DrawableRes
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import app.smartexplorer.android.R
import app.smartexplorer.android.core.VolumeInfo
import app.smartexplorer.android.system.Storage

/**
 * Display helpers for job endpoints. Locations stay opaque strings (api.md §2); these functions
 * only shorten them for cards and fields and never build a location.
 */
internal object SyncLocations {
    private val URL = Regex("^([a-z][a-z0-9+.-]*)://(?:[^@/]*@)?([^/]*)(/.*)?$", RegexOption.IGNORE_CASE)

    /** "Interner Speicher › DCIM/Camera", "SFTP host › /srv/data", "Google Drive › /Fotos", "Share › …". */
    fun label(location: String, volumes: List<VolumeInfo>): String {
        if (location.isBlank()) return "Nicht gewählt"
        if (location.startsWith("/")) return localLabel(location, volumes)
        val match = URL.find(location) ?: return location
        val scheme = match.groupValues[1].lowercase()
        val host = match.groupValues[2]
        val path = match.groupValues[3].ifEmpty { "/" }
        return when (scheme) {
            "gdrive" -> "Google Drive › $path"
            "share" -> "Share › $host$path"
            else -> "${scheme.uppercase()} $host › $path"
        }
    }

    @DrawableRes
    fun icon(location: String): Int {
        val scheme = location.substringBefore("://", missingDelimiterValue = "").lowercase()
        return when {
            scheme.isEmpty() -> R.drawable.ic_storage
            scheme == "gdrive" -> R.drawable.ic_drive
            scheme == "share" && location.contains("://room", ignoreCase = true) -> R.drawable.ic_room
            scheme == "share" -> R.drawable.ic_device
            else -> R.drawable.ic_cloud
        }
    }

    /** Arrow for the job direction (`a2b`, `b2a`, `both`). */
    fun arrow(direction: String): String = when (direction) {
        "a2b" -> "→"
        "b2a" -> "←"
        else -> "⇄"
    }

    private fun localLabel(path: String, volumes: List<VolumeInfo>): String {
        val volume = volumes
            .filter { path == it.path || path.startsWith(it.path.trimEnd('/') + "/") }
            .maxByOrNull { it.path.length }
            ?: return path
        val rest = path.removePrefix(volume.path).trim('/')
        return if (rest.isEmpty()) volume.label else "${volume.label} › $rest"
    }
}

/** Mounted volumes for [SyncLocations.label], read once per composition site. */
@Composable
internal fun rememberVolumes(): List<VolumeInfo> {
    val context = LocalContext.current
    return remember(context) { Storage.volumes(context) }
}
