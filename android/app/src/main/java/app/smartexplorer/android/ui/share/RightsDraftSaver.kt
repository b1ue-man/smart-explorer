package app.smartexplorer.android.ui.share

import androidx.compose.runtime.saveable.Saver
import app.smartexplorer.android.api.Rights
import kotlinx.serialization.SerializationException
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

// Keeps the rights sheet's draft through a configuration change (rotation, dark mode, locale), so
// the choices a user made are not thrown back to the seed. The draft is plain Kotlin; it is saved
// as one JSON string (a type every saved-state bundle takes) and read back without Compose, which
// is what the JVM tests cover.

@Serializable
private data class SavedRightsDraft(
    val rights: Rights = Rights(),
    val known: List<String> = emptyList(),
    val presetKey: String? = null,
    val expiry: String = EXPIRY_NONE,
    val keptUntil: Long? = null,
)

private val draftJson = Json {
    encodeDefaults = true
    ignoreUnknownKeys = true
}

/** [draft] as the text the saved state keeps. */
internal fun encodeRightsDraft(draft: RightsDraft): String = draftJson.encodeToString(
    SavedRightsDraft.serializer(),
    SavedRightsDraft(draft.rights, draft.known.toList(), draft.presetKey, draft.expiry, draft.keptUntil),
)

/** The draft [text] was saved from, or `null` when it is not one (the sheet then starts from its seed). */
internal fun decodeRightsDraft(text: String): RightsDraft? = try {
    val saved = draftJson.decodeFromString(SavedRightsDraft.serializer(), text)
    RightsDraft(saved.rights, saved.known.toSet(), saved.presetKey, saved.expiry, saved.keptUntil)
} catch (e: SerializationException) {
    null
} catch (e: IllegalArgumentException) {
    null
}

/**
 * `rememberSaveable(stateSaver = RIGHTS_DRAFT_SAVER) { mutableStateOf(…) }` for the draft of the
 * rights sheet: the saver handles the draft itself, `rememberSaveable` keeps the state around it.
 */
internal val RIGHTS_DRAFT_SAVER = Saver<RightsDraft, String>(
    save = { draft -> encodeRightsDraft(draft) },
    restore = { saved -> decodeRightsDraft(saved) },
)
