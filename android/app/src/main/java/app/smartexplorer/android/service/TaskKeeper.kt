package app.smartexplorer.android.service

import android.app.Application
import android.content.Context
import android.content.Intent
import android.util.Log
import androidx.core.content.ContextCompat
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.api.SyncApi.displayText
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.TaskInfo
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/**
 * Keeps the process alive for running tasks (spec F8) by starting [TaskForegroundService]
 * (dataSync). Android allows that start only while the UI is visible or just being left, so it
 * starts the service (a) while the UI is visible and user tasks (every kind except `catchup`)
 * run, and (b) when the UI is left while tasks run or the daemon runs a job (`bg.status.activeJob`).
 * The service stops itself once nothing runs any more. Tasks against another device
 * ([keepCpuAwake]) also keep the CPU awake while they run (the service holds the wake lock).
 */
object TaskKeeper {
    private const val TAG = "SmartExplorerTasks"
    private val started = AtomicBoolean(false)

    /** Process-wide scope for keeper and service work that must outlive a service instance. */
    internal val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    @Volatile
    internal var uiVisible = false
        private set

    private val cpuTasks = CpuTasks()

    fun start(app: Application) {
        if (!started.compareAndSet(false, true)) return
        scope.launch {
            Core.tasks.map { keepsAlive(it) }.distinctUntilChanged().collect { busy ->
                if (busy && uiVisible) startService(app)
            }
        }
    }

    internal fun onUiVisible(context: Context, visible: Boolean) {
        uiVisible = visible
        val app = context.applicationContext
        if (keepsAlive(Core.tasks.value)) {
            startService(app)
            return
        }
        if (visible) return
        // Leaving the UI: a job of the daemon also needs the process (the start exemption lasts a
        // few seconds after the UI became invisible; `bg.status` is a cheap snapshot).
        scope.launch {
            val activeJob = try {
                SyncApi.status().activeJob
            } catch (e: CoreException) {
                Log.w(TAG, "bg.status failed: ${e.kind}: ${e.displayText()}")
                null
            }
            if (activeJob != null) startService(app)
        }
    }

    /** User tasks that need the process; catch-up runs belong to the worker. */
    internal fun keepsAlive(tasks: List<TaskInfo>): Boolean = tasks.any { it.isActive && it.kind != SyncApi.KIND_CATCH_UP }

    /**
     * [taskId] works against another device (remote analysis or duplicate search): until it ends,
     * the task service holds a partial wake lock, so a screen turned off does not suspend the phone
     * in the middle of it (spec FA1).
     */
    fun keepCpuAwake(taskId: String) = cpuTasks.keep(taskId)

    /** Whether a running task needs the CPU ([keepCpuAwake]). */
    internal fun needsCpu(tasks: List<TaskInfo>): Boolean = cpuTasks.needed(tasks)

    /** [Abbrechen] in the task notification: every running user task. */
    internal fun cancelUserTasks() {
        scope.launch {
            Core.tasks.value.filter { it.isActive && it.kind != SyncApi.KIND_CATCH_UP }.forEach { task ->
                try {
                    SyncApi.cancelTask(task.id)
                } catch (e: CoreException) {
                    Log.w(TAG, "task.cancel ${task.id} failed: ${e.kind}: ${e.displayText()}")
                }
            }
        }
    }

    /** `onTimeout` of the task service: the system ends the service, so every task stops cleanly. */
    internal fun cancelAllAfterTimeout() {
        scope.launch {
            try {
                SyncApi.cancelAllTasks()
            } catch (e: CoreException) {
                Log.w(TAG, "task.cancelAll failed: ${e.kind}: ${e.displayText()}")
            }
        }
    }

    private fun startService(context: Context) {
        try {
            ContextCompat.startForegroundService(context, Intent(context, TaskForegroundService::class.java))
        } catch (e: IllegalStateException) {
            // ForegroundServiceStartNotAllowedException (API 31+) is an IllegalStateException.
            Log.w(TAG, "task service not started", e)
        } catch (e: SecurityException) {
            Log.w(TAG, "task service not started", e)
        }
    }
}

/**
 * Ids of tasks that work against another device and keep the CPU awake while they run. Ids of
 * ended tasks are forgotten; an id the task list does not know yet stays, since its first event may
 * still be on the way.
 */
internal class CpuTasks {
    private val ids = MutableStateFlow<Set<String>>(emptySet())

    fun keep(taskId: String) {
        ids.update { it + taskId }
    }

    fun needed(tasks: List<TaskInfo>): Boolean {
        val kept = ids.value
        if (kept.isEmpty()) return false
        val ended = tasks.filter { it.id in kept && !it.isActive }.map { it.id }.toSet()
        if (ended.isNotEmpty()) ids.update { it - ended }
        return tasks.any { it.isActive && it.id in kept }
    }

    /** Ids still kept (tests). */
    fun kept(): Set<String> = ids.value
}
