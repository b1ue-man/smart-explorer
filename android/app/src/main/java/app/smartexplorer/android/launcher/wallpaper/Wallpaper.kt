package app.smartexplorer.android.launcher.wallpaper

import android.app.WallpaperManager
import android.content.Context
import android.content.Intent
import android.os.IBinder
import kotlin.math.max

/**
 * The system wallpaper behind the home screen (U35-29). The window theme shows it (`windowShowWallpaper`);
 * the launcher never reads the picture (no storage permission) and only moves it with the pages.
 */
object Wallpaper {
    /**
     * Tells the wallpaper how many pages there are and where the pager stands: [position] runs from 0 on the
     * first page to 1 on the last.
     */
    fun setOffsets(context: Context, windowToken: IBinder?, pages: Int, position: Float) {
        windowToken ?: return
        val manager = context.getSystemService(WallpaperManager::class.java) ?: return
        try {
            manager.setWallpaperOffsetSteps(1f / max(1, pages - 1), 1f)
            manager.setWallpaperOffsets(windowToken, position.coerceIn(0f, 1f), 0.5f)
        } catch (_: RuntimeException) {
            // The window is gone; nothing to move.
        }
    }

    /** „Hintergrund ändern“ of the settings sheet. */
    fun changeIntent(): Intent = Intent.createChooser(Intent(Intent.ACTION_SET_WALLPAPER), "Hintergrund ändern")
}
