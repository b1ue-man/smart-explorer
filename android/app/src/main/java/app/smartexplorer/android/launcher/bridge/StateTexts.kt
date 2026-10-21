package app.smartexplorer.android.launcher.bridge

/** How a tile shows a state (U40-9, pattern `desktop/core/tiles.rs`). */
enum class Tone { Neutral, Gray, Yellow, OutlineGreen, Progress, Green, Red }

/**
 * The texts of the state of a device, word for word as the table in U40-9 gives them. Shared by the tiles of the
 * launcher, the widget and the widget feed, so it lives in `bridge`: pure Kotlin without Android types. The
 * `last_seen` part of the desktop text for `offline` does not exist on Android (`share.status` carries no such
 * field), so only „Offline“ is shown (`docs/TODO.md`: `LAUNCHER-LAST-SEEN`).
 */
object StateTexts {
    const val NOT_CURRENT = " · nicht aktuell"
    const val GONE = "Nicht mehr vorhanden"

    fun text(code: String, lan: Boolean = false): String = when (code) {
        StatusCodes.OFFLINE -> "Offline"
        StatusCodes.WAITING -> "Wartet auf das Gerät"
        StatusCodes.WAITING_FOR_ACCESS -> "Warte auf Freigabe"
        StatusCodes.AVAILABLE -> if (lan) "Erreichbar im LAN" else "Erreichbar"
        StatusCodes.CONNECTING -> "Verbindet"
        StatusCodes.CONNECTED -> "Verbunden"
        StatusCodes.CONNECTED_DIRECT -> "Verbunden (direkt)"
        StatusCodes.CONNECTED_RELAY -> "Verbunden (Relay)"
        StatusCodes.FAILED -> "Fehler"
        StatusCodes.IDENTITY_CONFLICT -> "Identität geändert – prüfen"
        else -> "Status unbekannt – nicht verbunden"
    }

    fun tone(code: String): Tone = when (code) {
        StatusCodes.OFFLINE, StatusCodes.WAITING -> Tone.Gray
        StatusCodes.WAITING_FOR_ACCESS -> Tone.Yellow
        StatusCodes.AVAILABLE -> Tone.OutlineGreen
        StatusCodes.CONNECTING -> Tone.Progress
        StatusCodes.CONNECTED, StatusCodes.CONNECTED_DIRECT, StatusCodes.CONNECTED_RELAY -> Tone.Green
        StatusCodes.FAILED, StatusCodes.IDENTITY_CONFLICT -> Tone.Red
        else -> Tone.Neutral
    }

    /** [text] with „ · nicht aktuell“ behind it when the data come from the cache. */
    fun withStale(text: String, stale: Boolean): String = if (stale) text + NOT_CURRENT else text

    /** The vault state of U40-9: `locked`, `unlocked`, `unavailable`. */
    fun vault(state: String): String = when (state) {
        "unlocked" -> "Entsperrt"
        "unavailable" -> "Nicht verfügbar"
        else -> "Gesperrt"
    }
}
