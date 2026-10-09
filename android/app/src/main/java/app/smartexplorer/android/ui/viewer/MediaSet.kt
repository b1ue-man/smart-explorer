package app.smartexplorer.android.ui.viewer

import app.smartexplorer.android.core.Entry

// Which entries the media viewer steps through (plan docs/plaene/2026-10-09-medien-weiterschalten,
// C3). Galleries mix pictures and videos; music players step through the audio files of a folder.

/** Media that are browsed together. */
internal enum class MediaGroup { Visual, Audio }

/** Group of an `Entry.kind` (api.md §2), `null` for everything that is not a medium. */
internal fun mediaGroupOf(kind: String): MediaGroup? = when (kind) {
    "image", "video" -> MediaGroup.Visual
    "audio" -> MediaGroup.Audio
    else -> null
}

/**
 * Media of one viewer session: [items] in the order the list shows them, opened at [start].
 * [local]: the place is local storage (`fs.open`); otherwise each item is loaded first (`fs.fetch`).
 */
internal data class MediaSet(val items: List<Entry>, val start: Int, val local: Boolean)

/**
 * The media of [shown] (the entries the list currently shows, in its order) that belong to the
 * group of [opened]. `null` when [opened] is no medium. An [opened] entry that is not part of
 * [shown] is viewed alone.
 */
internal fun mediaSetFor(shown: List<Entry>, opened: Entry, local: Boolean): MediaSet? {
    if (opened.isDir) return null
    val group = mediaGroupOf(opened.kind) ?: return null
    val items = shown.filter { !it.isDir && mediaGroupOf(it.kind) == group }
    val start = items.indexOfFirst { it.location == opened.location }
    return if (start < 0) MediaSet(listOf(opened), 0, local) else MediaSet(items, start, local)
}
