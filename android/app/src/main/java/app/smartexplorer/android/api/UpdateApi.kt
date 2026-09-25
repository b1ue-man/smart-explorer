package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import kotlinx.serialization.Serializable

// Answers of api.md §4.10 (update) and the version/log part of §4.1 (sys.info, sys.errors,
// sys.crashLog) used by the settings, "Über" and the error log.

/** `update.check`; [current] is the installed `versionName`. */
@Serializable
data class UpdateInfo(
    val current: String = "",
    val latest: String = "",
    val available: Boolean = false,
    val notes: String? = null,
)

/** Result of the `update.download` task: the verified APK below `<cache>/update/`. */
@Serializable
data class UpdateDownload(val path: String, val version: String = "")

/** `sys.info` */
@Serializable
data class CoreInfo(val coreVersion: String = "", val dataDir: String = "", val cacheDir: String = "")

/** One entry of `sys.errors`. */
@Serializable
data class ErrorLogEntry(val timeMs: Long = 0, val action: String = "", val message: String = "")

/** Suspending wrappers for api.md §4.10; every call throws [CoreException] on errors. */
object UpdateApi {
    suspend fun check(): UpdateInfo = Core.request<UpdateInfo>("update.check")

    /** Downloads the APK and checks its SHA-256; the task result is an [UpdateDownload]. */
    suspend fun download(): String = Core.request<TaskId>("update.download").taskId

    @Serializable
    private data class TaskId(val taskId: String)
}

/** Version and error-log calls of api.md §4.1; every call throws [CoreException] on errors. */
object SysApi {
    suspend fun info(): CoreInfo = Core.request<CoreInfo>("sys.info")

    suspend fun errors(): List<ErrorLogEntry> = Core.request<List<ErrorLogEntry>>("sys.errors")

    suspend fun clearErrors() {
        Core.call("sys.clearErrors")
    }

    /** The core's crash log, empty when there is none. */
    suspend fun crashLog(): String = Core.request<Text>("sys.crashLog").text

    @Serializable
    private data class Text(val text: String = "")
}
