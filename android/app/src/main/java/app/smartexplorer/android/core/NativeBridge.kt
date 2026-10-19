package app.smartexplorer.android.core

import android.content.Context
import java.nio.ByteBuffer

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

    /**
     * This phone as screen host (SCR-M8): one encoded picture or still of [session] from the
     * direct buffer [src] (bytes `offset until offset + length`). `flags`: bit 0 key frame, bit 1
     * still (8 bytes width and height, then RGBA), bits 16–31 the encoder epoch of `configure`.
     * Returns 0 when taken, −1 for an unknown or ended session, −2 for an empty or too large
     * picture, −3 when the buffer could not be read. The buffer is read during the call only.
     */
    external fun screenPush(session: Long, src: ByteBuffer, offset: Int, length: Int, ptsUs: Long, flags: Int): Int

    /**
     * The next control message (JSON) of [session], session 0 for the input of the controlling
     * viewer; `null` when none came within [timeoutMs] (at most 5 s).
     */
    external fun screenControl(session: Long, timeoutMs: Int): String?

    /** Error envelope for a missing answer (see class comment). */
    fun orError(raw: String?): String =
        raw ?: "{\"err\":{\"kind\":\"internal\",\"message\":\"Der Kern lieferte keine Antwort\"}}"
}
