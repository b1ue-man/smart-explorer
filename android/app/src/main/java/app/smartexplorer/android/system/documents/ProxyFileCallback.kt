package app.smartexplorer.android.system.documents

import android.os.HandlerThread
import android.os.ProxyFileDescriptorCallback
import android.system.ErrnoException
import app.smartexplorer.android.api.DocsBackend

/**
 * One proxy descriptor another app holds (DRV-M8, strand S2 C3/C4). Its callbacks run on the
 * file's own [thread], so a slow file never holds up another one. `onGetSize` answers from the
 * handle without the network (vold and the system_server watchdog wait on it, NR N11);
 * `onFsync` publishes and reports its error; `onRelease` never blocks: the core publishes in the
 * background and [onReleased] queues the commit worker, which retries after a process end.
 */
class ProxyFileCallback(
    private val backend: DocsBackend,
    private val handle: Long,
    private val thread: HandlerThread,
    private val onReleased: () -> Unit,
) : ProxyFileDescriptorCallback() {
    private fun check(name: String, result: Long): Long {
        if (result < 0) throw ErrnoException(name, (-result).toInt())
        return result
    }

    override fun onGetSize(): Long = check("onGetSize", backend.size(handle))

    override fun onRead(offset: Long, size: Int, data: ByteArray): Int =
        check("onRead", backend.read(handle, offset, size, data).toLong()).toInt()

    override fun onWrite(offset: Long, size: Int, data: ByteArray): Int =
        check("onWrite", backend.write(handle, offset, size, data).toLong()).toInt()

    override fun onFsync() {
        check("onFsync", backend.fsync(handle).toLong())
    }

    override fun onRelease() {
        backend.release(handle)
        thread.quitSafely()
        onReleased()
    }
}
