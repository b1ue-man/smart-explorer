package app.smartexplorer.android.core

/** Events delivered by `pollEvents` (api.md §3). */
sealed interface CoreEvent {
    /** Task progress (bundled, at most 4/s per task) and completion. */
    data class Task(val task: TaskInfo) : CoreEvent

    /** Share state changed: reload `share.status`. */
    data object Share : CoreEvent

    /** New incoming share request(s). */
    data class ShareRequest(val count: Int) : CoreEvent

    /** Sync jobs or their results changed: reload `sync.jobs`. */
    data object Jobs : CoreEvent

    /** An opened remote copy was changed locally: reload `fs.edits`. */
    data object Edits : CoreEvent

    /** The core wants a URL opened in the browser (OAuth). */
    data class OpenUrl(val url: String) : CoreEvent

    /** New entry for the error log. */
    data class Error(val action: String, val message: String) : CoreEvent

    /** Storage volumes changed. */
    data object Volumes : CoreEvent
}

/** Start state of the core, see [Core.ready]. */
sealed interface CoreState {
    data object Starting : CoreState

    data object Ready : CoreState

    data class Failed(val message: String) : CoreState
}
