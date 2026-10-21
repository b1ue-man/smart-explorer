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

    /**
     * This phone as screen viewer (SCR-M9): the next picture of [session] into the direct buffer
     * [dst] (the decoder's input buffer), little endian: bytes 0–7 capture time in µs, 8–11 flags
     * (bit 0 key frame, bit 1 finished tile picture R,G,B,A, bit 2 first picture of a new group,
     * bit 3 complete; bits 16–31 the low 16 bits of the group number), 12–15 payload length, then
     * the payload from byte 16. Returns the bytes written, −1 when none came within [timeoutMs]
     * (at most 1000), −2 when the session ended, −3 when [dst] is too small (bytes 12–15 then name
     * the payload length, nothing else is written and the picture stays queued) and −4 when the
     * buffer is not direct.
     */
    external fun screenPull(session: Long, dst: ByteBuffer, timeoutMs: Int): Int

    /*
     * Documents provider (DRV-M8): the proxy descriptors other apps hold. Each call returns a
     * negated errno on failure (SeDocumentsProvider throws ErrnoException with it).
     */

    /** Opens [documentId] in [mode] (`r`, `w`, `wt`, `rw`, `rwt`): handle ≥ 1 or −errno. */
    external fun docOpen(documentId: String, mode: String): Long

    /** Reads up to [size] bytes at [offset] into [data]: bytes read (0 at the end) or −errno. */
    external fun docRead(handle: Long, offset: Long, size: Int, data: ByteArray): Int

    /** Writes [size] bytes of [data] at [offset]: bytes written or −errno. */
    external fun docWrite(handle: Long, offset: Long, size: Int, data: ByteArray): Int

    /** The size from the handle, never over the network: bytes or −errno. */
    external fun docSize(handle: Long): Long

    /** Publishes the content now: 0 or −errno. */
    external fun docFsync(handle: Long): Int

    /** Ends the handle without blocking; publishing continues in the background: 0 or −errno. */
    external fun docRelease(handle: Long): Int

    /** Error envelope for a missing answer (see class comment). */
    fun orError(raw: String?): String =
        raw ?: "{\"err\":{\"kind\":\"internal\",\"message\":\"Der Kern lieferte keine Antwort\"}}"
}
