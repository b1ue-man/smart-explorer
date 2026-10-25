package app.smartexplorer.android.screen

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.drawable.BitmapDrawable
import android.util.Base64
import java.io.ByteArrayOutputStream
import java.security.MessageDigest
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

/** Same reference on the native wire and in the Android bridge. No bitmap belongs in a list. */
@Serializable
data class ScreenIconRef(
    val version: Int = 1,
    @SerialName("app_id") val appId: String,
    val sha256: String,
    val width: Int,
    val height: Int,
)

@Serializable
data class ScreenIconReply(
    val icon: ScreenIconRef,
    @SerialName("png_base64") val pngBase64: String,
)

@Serializable
data class ScreenIconTarget(
    val kind: String,
    val id: String? = null,
    @SerialName("app_id") val appId: String? = null,
) {
    companion object {
        fun window(id: String) = ScreenIconTarget("window", id = id)
        fun app(appId: String) = ScreenIconTarget("app", appId = appId)
    }
}

@Serializable
data class ScreenIconRequest(val target: ScreenIconTarget, val icon: ScreenIconRef)

/** Only installed PackageManager identities enter this source; the network never supplies a path. */
object AppIcons {
    private const val SIDE = 128
    private const val MAX_BYTES = 256 * 1024
    private const val MAX_PACKAGES = 128
    private val published = LinkedHashMap<String, ScreenIconReply>()

    /** Re-render actual app bytes on refresh so package/theme changes cannot retain a name cache. */
    fun reference(context: Context, packageName: String): ScreenIconRef? {
        val reply = render(context, packageName)
        synchronized(published) {
            published.remove(packageName)
            if (reply != null) published[packageName] = reply
            while (published.size > MAX_PACKAGES) published.remove(published.keys.first())
        }
        return reply?.icon
    }

    /** Local hostPrereqs uses this after its current RunningApps snapshot. */
    fun contentFor(reference: ScreenIconRef): ScreenIconReply? = synchronized(published) {
        published[reference.appId]?.takeIf { it.icon == reference }
    }

    fun retainPackages(packages: Set<String>) = synchronized(published) {
        published.keys.retainAll(packages)
    }

    private fun render(context: Context, packageName: String): ScreenIconReply? = try {
        val pm = context.packageManager
        val app = pm.getApplicationInfo(packageName, 0)
        // A missing icon is the neutral UI fallback, not PackageManager's generic default app.
        if (app.icon == 0) null else {
            val drawable = pm.getApplicationIcon(app)
            val oversized = drawable is BitmapDrawable &&
                (drawable.bitmap.width > 2048 || drawable.bitmap.height > 2048 ||
                    drawable.bitmap.width.toLong() * drawable.bitmap.height > 4_194_304L)
            if (oversized) null else {
                val bitmap = Bitmap.createBitmap(SIDE, SIDE, Bitmap.Config.ARGB_8888)
                try {
                    drawable.setBounds(0, 0, SIDE, SIDE)
                    drawable.draw(Canvas(bitmap))
                    val output = ByteArrayOutputStream()
                    if (!bitmap.compress(Bitmap.CompressFormat.PNG, 100, output)) null else {
                        val png = output.toByteArray()
                        if (png.size > MAX_BYTES) null else {
                            val hash = MessageDigest.getInstance("SHA-256").digest(png)
                                .joinToString("") { "%02x".format(it.toInt() and 255) }
                            ScreenIconReply(ScreenIconRef(1, packageName, hash, SIDE, SIDE), Base64.encodeToString(png, Base64.NO_WRAP))
                        }
                    }
                } finally { bitmap.recycle() }
            }
        }
    } catch (_: Exception) { null }
}
