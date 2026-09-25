package app.smartexplorer.android.core

import android.content.Context

/**
 * JNI entry points of `libsmart_explorer_android.so` (api.md §1). Every function returns UTF-8
 * JSON (`{"ok": …}` or `{"err": {"kind", "message"}}`) and never throws a Java exception.
 * Only if even building the answer string fails (out of memory) the bridge returns `null`;
 * [orError] turns that into an error envelope. Call only through [Core].
 */
object NativeBridge {
    external fun init(context: Context, configJson: String): String?

    external fun call(method: String, argsJson: String): String?

    external fun pollEvents(timeoutMs: Int): String?

    /** Error envelope for a missing answer (see class comment). */
    fun orError(raw: String?): String =
        raw ?: "{\"err\":{\"kind\":\"internal\",\"message\":\"Der Kern lieferte keine Antwort\"}}"
}
