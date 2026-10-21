package app.smartexplorer.android.launcher.apps

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.drawable.Drawable
import android.util.LruCache
import java.io.File
import java.io.IOException
import java.security.MessageDigest
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Icons as bitmaps of `iconDp` times the screen density (U35-24): `ARGB_8888`, drawn from the activity's
 * icon (with the work badge for another profile), cached in memory (`maxMemory / 8` bytes) and as PNG in
 * `<cacheDir>/launcher-icons/`. Loading runs off the main thread; until it is done the caller draws an
 * empty circle.
 */
class IconLoader private constructor(private val context: Context) {
    private val dir = File(context.cacheDir, "launcher-icons")
    private val memory = object : LruCache<String, Bitmap>((Runtime.getRuntime().maxMemory() / 8).coerceIn(1L, Int.MAX_VALUE.toLong()).toInt()) {
        override fun sizeOf(key: String, value: Bitmap): Int = value.byteCount
    }

    val densityDpi: Int get() = context.resources.displayMetrics.densityDpi

    fun pixels(iconDp: Int): Int = (iconDp * context.resources.displayMetrics.density).toInt().coerceAtLeast(1)

    /** The icon already in memory, if any. */
    fun peek(key: String): Bitmap? = memory.get(key)

    /** Key of an app icon: `<package>~<class hash>~<user serial>~<version>~<pixels>` (U35-24). */
    fun appKey(entry: AppEntry, px: Int): String =
        "${entry.packageName}~${hash8(entry.component.className)}~${entry.userSerial}~${entry.versionCode}~$px"

    /** Key of a shortcut icon; memory only, since shortcut icons change without a new version. */
    fun shortcutKey(packageName: String, shortcutId: String, userSerial: Long, px: Int): String =
        "$packageName~sc~${hash8(shortcutId)}~$userSerial~$px"

    suspend fun appIcon(entry: AppEntry, px: Int): Bitmap? =
        load(appKey(entry, px), px, persist = true) {
            val drawable = entry.info.getIcon(densityDpi)
            // A profile other than the launcher's gets its badge from the system.
            if (entry.kind == ProfileKind.Personal) drawable else context.packageManager.getUserBadgedIcon(drawable, entry.user)
        }

    suspend fun load(key: String, px: Int, persist: Boolean, drawable: () -> Drawable?): Bitmap? {
        memory.get(key)?.let { return it }
        return withContext(Dispatchers.IO) {
            memory.get(key)?.let { return@withContext it }
            val file = File(dir, "$key.png")
            val cached = if (persist && file.isFile) BitmapFactory.decodeFile(file.path) else null
            val bitmap = cached ?: draw(drawable(), px)?.also { if (persist) write(file, it) }
            if (bitmap != null) memory.put(key, bitmap)
            bitmap
        }
    }

    /** Forgets everything of [packageName]: memory and files (package changed or removed, U35-29). */
    fun invalidate(packageName: String) {
        val prefix = "$packageName~"
        memory.snapshot().keys.filter { it.startsWith(prefix) }.forEach { memory.remove(it) }
        dir.listFiles { file -> file.name.startsWith(prefix) }?.forEach { it.delete() }
    }

    private fun draw(drawable: Drawable?, px: Int): Bitmap? {
        drawable ?: return null
        val bitmap = Bitmap.createBitmap(px, px, Bitmap.Config.ARGB_8888)
        drawable.setBounds(0, 0, px, px)
        drawable.draw(Canvas(bitmap))
        return bitmap
    }

    private fun write(file: File, bitmap: Bitmap) {
        try {
            dir.mkdirs()
            file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        } catch (_: IOException) {
            file.delete()
        }
    }

    private fun hash8(text: String): String =
        MessageDigest.getInstance("SHA-256").digest(text.toByteArray(Charsets.UTF_8)).take(4).joinToString("") { "%02x".format(it) }

    companion object {
        @Volatile
        private var instance: IconLoader? = null

        fun get(context: Context): IconLoader = instance ?: synchronized(this) {
            instance ?: IconLoader(context.applicationContext).also { instance = it }
        }
    }
}
