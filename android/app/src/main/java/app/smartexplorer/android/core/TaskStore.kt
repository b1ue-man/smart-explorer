package app.smartexplorer.android.core

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

/** Current task list, fed by `task` events and `task.list` snapshots. */
internal class TaskStore {
    private val state = MutableStateFlow<List<TaskInfo>>(emptyList())
    val tasks: StateFlow<List<TaskInfo>> = state.asStateFlow()

    fun upsert(task: TaskInfo) {
        state.update { list ->
            val index = list.indexOfFirst { it.id == task.id }
            if (index < 0) list + task else list.toMutableList().also { it[index] = task }
        }
    }

    /**
     * Applies a `task.list` snapshot. Events may overtake the snapshot while it travels, so a
     * finished state already seen by an event wins over a still-running snapshot entry, and tasks
     * that started after the snapshot was taken stay in the list.
     */
    fun replaceAll(snapshot: List<TaskInfo>) {
        state.update { current ->
            val known = current.associateBy { it.id }
            val merged = snapshot.map { fresh ->
                val seen = known[fresh.id]
                if (seen != null && !seen.isActive && fresh.isActive) seen else fresh
            }
            val snapshotIds = snapshot.mapTo(HashSet()) { it.id }
            merged + current.filter { it.isActive && it.id !in snapshotIds }
        }
    }
}
