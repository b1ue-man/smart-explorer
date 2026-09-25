package app.smartexplorer.android.core

import android.content.Context

/**
 * JNI entry points of `libsmart_explorer_android.so` (api.md §1). Every function returns UTF-8
 * JSON (`{"ok": …}` or `{"err": {"kind", "message"}}`) and never throws a Java exception.
 * The library is loaded by [Core.start]; call only through [Core].
 */
object NativeBridge {
    external fun init(context: Context, configJson: String): String

    external fun call(method: String, argsJson: String): String

    external fun pollEvents(timeoutMs: Int): String
}
