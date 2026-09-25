package app.smartexplorer.android.system

import android.content.Context
import android.os.Environment
import android.os.storage.StorageManager
import android.os.storage.StorageVolume
import app.smartexplorer.android.core.VolumeInfo

/** Mounted storage volumes as file paths (android-apis.md §5.2). */
object Storage {
    /** Mounted volumes with a readable mount point; the primary (internal) volume first. */
    fun volumes(context: Context): List<VolumeInfo> {
        val manager = context.getSystemService(StorageManager::class.java) ?: return emptyList()
        return manager.storageVolumes
            .filter { it.state == Environment.MEDIA_MOUNTED || it.state == Environment.MEDIA_MOUNTED_READ_ONLY }
            .mapNotNull { volume ->
                val dir = volume.directory ?: return@mapNotNull null
                VolumeInfo(
                    path = dir.absolutePath,
                    label = volume.getDescription(context) ?: dir.name,
                    primary = volume.isPrimary,
                    removable = volume.isRemovable,
                )
            }
            .sortedByDescending { it.primary }
    }

    /**
     * Calls [onChange] on the main thread whenever a volume changes state (mounted, ejected, …).
     * Registered once for the process lifetime by [app.smartexplorer.android.core.Core].
     */
    fun watchVolumes(context: Context, onChange: () -> Unit) {
        val manager = context.getSystemService(StorageManager::class.java) ?: return
        manager.registerStorageVolumeCallback(
            context.mainExecutor,
            object : StorageManager.StorageVolumeCallback() {
                override fun onStateChanged(volume: StorageVolume) = onChange()
            },
        )
    }
}
