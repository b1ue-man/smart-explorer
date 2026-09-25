package app.smartexplorer.android.core

/**
 * Error reported by the core (`{"err": {"kind", "message"}}`, api.md §1). [kind] is one of
 * `not_found, permission, exists, invalid, unsupported, network, auth, conflict, busy, canceled,
 * not_initialized, internal`; the message is German display text.
 */
class CoreException(val kind: String, message: String) : Exception(message)
