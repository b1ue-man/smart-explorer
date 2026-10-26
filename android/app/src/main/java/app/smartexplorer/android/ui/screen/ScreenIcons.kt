package app.smartexplorer.android.ui.screen

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.Base64
import android.util.LruCache
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Apps
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.produceState
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.screen.ScreenIconRef
import app.smartexplorer.android.screen.ScreenIconReply
import app.smartexplorer.android.screen.ScreenIconRequest
import app.smartexplorer.android.screen.ScreenIconTarget
import java.security.MessageDigest
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit

private data class IconKey(val peer: String, val icon: ScreenIconRef)

/** Cache retention is bounded by decoded bytes, with authenticated peer/app/hash keys. */
object ScreenIconContent {
    private const val MAX_BYTES = 256 * 1024
    private val requests = Semaphore(8)
    suspend fun fetch(request: ScreenIconRequest, load: suspend (ScreenIconRequest) -> ScreenIconReply?): ScreenIconReply? =
        requests.withPermit { load(request) }
    private val cache = object : LruCache<IconKey, Bitmap>(8 * 1024 * 1024) {
        override fun sizeOf(key: IconKey, value: Bitmap) = value.allocationByteCount
    }

    fun forgetPeer(peer: String) = synchronized(cache) {
        cache.snapshot().keys.filter { it.peer == peer }.forEach { cache.remove(it) }
    }

    fun lookup(peer: String, icon: ScreenIconRef): Bitmap? = synchronized(cache) { cache.get(IconKey(peer, icon)) }

    private fun staticPng(bytes: ByteArray): Boolean {
        var at = 8
        while (at <= bytes.size - 12) {
            val size = (0..3).fold(0L) { value, index -> (value shl 8) or (bytes[at + index].toLong() and 255L) }
            if (size > bytes.size - at - 12L) return false
            val kind = String(bytes, at + 4, 4, Charsets.US_ASCII)
            if (kind == "acTL" || kind == "fcTL" || kind == "fdAT") return false
            at += size.toInt() + 12
            if (kind == "IEND") return size == 0L && at == bytes.size
        }
        return false
    }

    fun accept(peer: String, expected: ScreenIconRef, reply: ScreenIconReply): Bitmap? {
        if (peer.isEmpty() || expected != reply.icon || expected.version != 1 ||
            expected.appId.isEmpty() || expected.appId.length > 1024 ||
            !expected.sha256.matches(Regex("[0-9a-f]{64}")) ||
            expected.width !in 1..256 || expected.height !in 1..256 ||
            expected.width.toLong() * expected.height > 65_536L ||
            reply.pngBase64.length > ((MAX_BYTES + 2) / 3) * 4) return null
        val bytes = try { Base64.decode(reply.pngBase64, Base64.NO_WRAP) } catch (_: IllegalArgumentException) { return null }
        if (bytes.size !in 33..MAX_BYTES || !bytes.copyOfRange(0, 8).contentEquals(byteArrayOf(-119,80,78,71,13,10,26,10)) ||
            !bytes.copyOfRange(8, 16).contentEquals(byteArrayOf(0,0,0,13,73,72,68,82)) || !staticPng(bytes)) return null
        val hash = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it.toInt() and 255) }
        if (hash != expected.sha256) return null
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        if (bounds.outWidth != expected.width || bounds.outHeight != expected.height || bounds.outMimeType != "image/png") return null
        val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size,
            BitmapFactory.Options().apply { inPreferredConfig = Bitmap.Config.ARGB_8888; inScaled = false }) ?: return null
        if (bitmap.width != expected.width || bitmap.height != expected.height || bitmap.allocationByteCount > MAX_BYTES) {
            bitmap.recycle()
            return null
        }
        synchronized(cache) {
            cache.snapshot().keys.filter { it.peer == peer && it.icon.appId == expected.appId && it.icon != expected }.forEach { cache.remove(it) }
            cache.put(IconKey(peer, expected), bitmap)
        }
        return bitmap
    }
}

/** `peer` includes relation, principal and device; fetch binds the current listed target to it. */
@Composable
fun ScreenAppIcon(
    peer: String,
    target: ScreenIconTarget,
    reference: ScreenIconRef?,
    modifier: Modifier = Modifier.size(24.dp),
    fetch: suspend (ScreenIconRequest) -> ScreenIconReply?,
) {
    // A changed peer/app starts with fresh state, so a former peer's icon cannot flash.
    val bitmap by key(peer, target, reference) {
        produceState<Bitmap?>(null) {
            value = reference?.let { ScreenIconContent.lookup(peer, it) }
            if (value == null && reference != null) {
                value = try {
                    val reply = ScreenIconContent.fetch(ScreenIconRequest(target, reference), fetch)
                    if (reply == null) null else withContext(Dispatchers.Default) { ScreenIconContent.accept(peer, reference, reply) }
                } catch (cancelled: CancellationException) { throw cancelled } catch (_: Exception) { null }
            }
        }
    }
    if (bitmap != null) Image(bitmap!!.asImageBitmap(), contentDescription = null, modifier = modifier)
    else Icon(Icons.Outlined.Apps, contentDescription = "Programmsymbol nicht verfügbar", modifier = modifier)
}
