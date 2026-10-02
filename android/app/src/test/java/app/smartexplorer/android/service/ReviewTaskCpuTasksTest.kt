package app.smartexplorer.android.service

import app.smartexplorer.android.core.TaskInfo
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * RV1 FA1: a task against another device keeps the CPU awake exactly while it runs; ended tasks
 * are forgotten, a task the list does not know yet stays registered.
 */
class ReviewTaskCpuTasksTest {
    private fun task(id: String, state: String) = TaskInfo(id = id, kind = "analyze", title = id, state = state)

    @Test
    fun cpuIsNeededOnlyWhileARegisteredTaskRuns() {
        val cpu = CpuTasks()
        assertFalse(cpu.needed(listOf(task("local", "running"))))

        cpu.keep("remote")
        assertFalse("not in the list yet", cpu.needed(emptyList()))
        assertEquals(setOf("remote"), cpu.kept())
        assertTrue(cpu.needed(listOf(task("local", "running"), task("remote", "running"))))
        assertTrue(cpu.needed(listOf(task("remote", "queued"))))

        assertFalse(cpu.needed(listOf(task("remote", "done"))))
        assertTrue("an ended task is forgotten", cpu.kept().isEmpty())
        assertFalse(cpu.needed(listOf(task("remote", "running"))))
    }
}
