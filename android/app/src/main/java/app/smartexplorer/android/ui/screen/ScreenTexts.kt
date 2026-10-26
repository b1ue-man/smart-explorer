package app.smartexplorer.android.ui.screen

import android.content.Context
import app.smartexplorer.android.R

/**
 * The German texts of a viewer session (SCR-M9, U25-9): the same words as the desktop viewer
 * (`strings.xml`, `screen_<kind>_<key>`). Every state, error code, end reason and input block the
 * core names has its own text; a code of a newer host stays visible as „Fehler des Geräts: {Code}“.
 * The `…Res` functions give the resource of a key (`null` = unknown key); the keys of the core are
 * listed in `src/test/resources/screen_keys.txt`.
 */
object ScreenTexts {
    fun stateRes(key: String): Int? = when (key) {
        "connecting" -> R.string.screen_state_connecting
        "waiting_for_approval" -> R.string.screen_state_waiting_for_approval
        "waiting_for_portal" -> R.string.screen_state_waiting_for_portal
        "starting" -> R.string.screen_state_starting
        "streaming" -> R.string.screen_state_streaming
        "secure_desktop" -> R.string.screen_state_secure_desktop
        "window_minimized" -> R.string.screen_state_window_minimized
        "host_locked" -> R.string.screen_state_host_locked
        "session_inactive" -> R.string.screen_state_session_inactive
        "no_display" -> R.string.screen_state_no_display
        "ended" -> R.string.screen_state_ended
        "unknown" -> R.string.screen_state_unknown
        else -> null
    }

    fun errorRes(key: String): Int? = when (key) {
        "permission_denied" -> R.string.screen_error_permission_denied
        "right_not_granted" -> R.string.screen_error_right_not_granted
        "approval_denied" -> R.string.screen_error_approval_denied
        "approval_cancelled" -> R.string.screen_error_approval_cancelled
        "no_display" -> R.string.screen_error_no_display
        "indicator_unavailable" -> R.string.screen_error_indicator_unavailable
        "control_busy" -> R.string.screen_error_control_busy
        "control_unavailable" -> R.string.screen_error_control_unavailable
        "host_shutdown" -> R.string.screen_error_host_shutdown
        "screen_unsupported" -> R.string.screen_error_screen_unsupported
        "approval_unavailable" -> R.string.screen_error_approval_unavailable
        "capture_failed" -> R.string.screen_error_capture_failed
        "encoder_failed" -> R.string.screen_error_encoder_failed
        "session_limit" -> R.string.screen_error_session_limit
        else -> null
    }

    fun endRes(key: String): Int? = when (key) {
        "viewer_closed" -> R.string.screen_end_viewer_closed
        "host_stopped" -> R.string.screen_end_host_stopped
        "capability_narrowed" -> R.string.screen_end_capability_narrowed
        "grant_expired" -> R.string.screen_end_grant_expired
        "host_shutdown" -> R.string.screen_end_host_shutdown
        "update_stop" -> R.string.screen_end_update_stop
        "connection_lost" -> R.string.screen_end_connection_lost
        "failed" -> R.string.screen_end_failed
        "ended" -> R.string.screen_end_ended
        "connection_failed" -> R.string.screen_end_connection_failed
        else -> null
    }

    fun blockRes(key: String): Int? = when (key) {
        "elevated_window" -> R.string.screen_block_elevated_window
        "secure_desktop" -> R.string.screen_block_secure_desktop
        "control_unavailable" -> R.string.screen_block_control_unavailable
        "unsupported_on_host" -> R.string.screen_block_unsupported_on_host
        "other" -> R.string.screen_block_other
        else -> null
    }

    private fun reasonRes(key: String): Int? = when (key) {
        "pipewire_missing" -> R.string.screen_reason_pipewire_missing
        "capture_unavailable" -> R.string.screen_reason_capture_unavailable
        "window_capture_unsupported" -> R.string.screen_reason_window_capture_unsupported
        "portal_denied" -> R.string.screen_reason_portal_denied
        "monitor_not_found" -> R.string.screen_reason_monitor_not_found
        "window_not_found" -> R.string.screen_reason_window_not_found
        "portal_selects_source" -> R.string.screen_reason_portal_selects_source
        "source_identity_unavailable" -> R.string.screen_reason_source_identity_unavailable
        "no_display" -> R.string.screen_reason_no_display
        else -> null
    }

    /** [base] with `: detail` when the host gave a diagnostic text. */
    internal fun withDetail(base: String, detail: String?): String {
        val text = detail?.trim().orEmpty()
        return if (text.isEmpty()) base else "$base: $text"
    }

    /** The status line of a session in state [key]; empty while it streams. */
    fun stateText(context: Context, key: String, reason: String? = null): String {
        val res = stateRes(key) ?: return context.getString(R.string.screen_state_unknown)
        val base = context.getString(res)
        return when (key) {
            "no_display" -> reason?.takeIf { it.isNotEmpty() }
                ?.let { withDetail(base, reasonRes(it)?.let(context::getString) ?: it) } ?: base
            "ended" -> reason?.takeIf { it.isNotEmpty() }?.let { endText(context, it, null) } ?: base
            else -> base
        }
    }

    /** The text of an error code of the host (`HELLO_ERROR`, `ERROR`). */
    fun errorText(context: Context, code: String, detail: String? = null): String {
        val res = errorRes(code) ?: return context.getString(R.string.screen_error_unknown, code)
        val base = context.getString(res)
        return if (code == "capture_failed" || code == "encoder_failed") withDetail(base, detail) else base
    }

    /** The text of the end of a session (`ended` event: a reason key or an error code). */
    fun endText(context: Context, code: String, detail: String? = null): String {
        val res = endRes(code) ?: return errorText(context, code, detail)
        val base = context.getString(res)
        return if (code == "connection_failed" || code == "failed") withDetail(base, detail) else base
    }

    /** Why input does not reach the host. */
    fun blockedText(context: Context, key: String): String =
        context.getString(blockRes(key) ?: R.string.screen_block_other)
}
