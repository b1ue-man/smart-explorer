package app.smartexplorer.android.task

import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.work.Configuration
import androidx.work.NetworkType
import androidx.work.WorkInfo
import androidx.work.WorkManager
import androidx.work.testing.SynchronousExecutor
import androidx.work.testing.WorkManagerTestInitHelper
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.service.BackgroundController
import java.io.File
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.delay
import kotlinx.serialization.builtins.ListSerializer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * G4 background: the embedded daemon controls (`bg.*`), a catch-up run that admits a due job,
 * and the periodic `SyncWorker` driven by WorkManager's TestDriver (constraints and period).
 */
@RunWith(AndroidJUnit4::class)
class BackgroundTaskTest {
    private suspend fun catchUpTasks(): List<TaskInfo> =
        Core.json.decodeFromJsonElement(ListSerializer(TaskInfo.serializer()), Api.call("task.list")).filter { it.kind == "catchup" }

    @Test
    fun daemonControlsPauseAutopauseAndLog() = coreTest {
        assertTrue(Api.obj("bg.ensureDaemon").bool("running"))
        assertTrue(Api.obj("bg.status").bool("daemonRunning"))

        Api.call("bg.setSyncEnabled", args("enabled" to false))
        assertFalse(Api.obj("bg.status").bool("syncEnabled"))
        val off = Api.await(Api.start("bg.catchUp"))
        assertEquals("catchup", off.kind)
        assertNotNull("Nachhol-Lauf bei Sync aus ohne Meldung", off.message)
        Api.call("bg.setSyncEnabled", args("enabled" to true))
        assertTrue(Api.obj("bg.status").bool("syncEnabled"))

        Api.call("bg.pause", args("seconds" to -1))
        val unlimited = Api.obj("bg.status")
        assertTrue(unlimited.bool("paused"))
        assertNull(unlimited.textOrNull("pausedUntilMs"))
        Api.call("bg.resume")
        assertFalse(Api.obj("bg.status").bool("paused"))
        Api.call("bg.pause", args("seconds" to 3600))
        assertTrue(Api.obj("bg.status").long("pausedUntilMs") > System.currentTimeMillis())
        Api.call("bg.resume")

        Api.call("bg.setAutopause", args("battery" to true, "metered" to true))
        val auto = Api.obj("bg.status")
        assertTrue(auto.bool("autopauseBattery") && auto.bool("autopauseMetered"))
        Api.call("bg.setAutopause", args("battery" to false, "metered" to false))
        assertTrue(Api.obj("bg.status").int("cadenceSecs") > 0)
        Api.obj("bg.log", args("maxBytes" to 4096)).text("text")
    }

    @Test
    fun catchUpAdmitsADueIntervalJob() = coreTest {
        Api.obj("bg.ensureDaemon")
        Api.call("bg.setSyncEnabled", args("enabled" to true))
        Api.call("bg.resume")
        val source = Fixture.dir(Volumes.primary(), "background", "quelle")
        val target = Fixture.dir(Volumes.primary(), "background", "ziel")
        Fixture.write(File(source, "nachholen.txt"), "fällig")
        val name = Fixture.unique("Nachhol-Job")
        val job = SyncJobs.save(
            "name" to name,
            "source" to source.absolutePath,
            "target" to target.absolutePath,
            "direction" to "a2b",
            "trigger" to "interval",
            "intervalMin" to 5,
            "catchUp" to true,
            "enabled" to true,
        )
        val run = Api.await(Api.start("bg.catchUp"), timeoutMs = 300_000)
        assertEquals("catchup", run.kind)
        val result = run.resultObj()
        val admitted = result.int("admitted")
        val skipped = result.objects("skipped")
        TaskReport.note("bg.catchUp", "state=${run.state} message=${run.message} result=$result")
        // The daemon may already have run the due job itself; then the run lists it as skipped.
        assertTrue("Job weder zugelassen noch mit Grund übersprungen: $result", admitted >= 1 || skipped.any { it.text("jobName") == name })
        val deadline = System.currentTimeMillis() + 120_000
        while (!File(target, "nachholen.txt").isFile && System.currentTimeMillis() < deadline) delay(500)
        assertTrue("Fälliger Job lief nicht", File(target, "nachholen.txt").isFile)
        assertNotNull(Api.obj("bg.status").textOrNull("lastCatchUpMs"))
        Api.call("sync.delete", args("id" to job.text("id")))
    }

    @Test
    fun periodicWorkerRunsCatchUpWhenConstraintsAndPeriodAreMet() = coreTest {
        val context = appContext
        Api.obj("bg.ensureDaemon")
        Api.call("bg.resume")
        // The real periodic work of the app start must not interfere: cancel it, then switch to
        // WorkManager's test implementation for the rest of this process.
        WorkManager.getInstance(context).cancelUniqueWork(BackgroundController.WORK_NAME).result.get(30, TimeUnit.SECONDS)
        val config = Configuration.Builder().setMinimumLoggingLevel(Log.DEBUG).setExecutor(SynchronousExecutor()).build()
        WorkManagerTestInitHelper.initializeTestWorkManager(context, config)
        val driver = WorkManagerTestInitHelper.getTestDriver(context) ?: throw AssertionError("WorkManager TestDriver fehlt")

        AppPrefs.setBgMode(BackgroundController.MODE_PERIODIC)
        AppPrefs.setBgIntervalMin(60)
        AppPrefs.setBgWifiOnly(true)
        AppPrefs.setBgChargingOnly(false)
        AppPrefs.setBgBatteryNotLow(true)
        val before = catchUpTasks().map { it.id }.toSet()
        BackgroundController.apply(context)

        val workManager = WorkManager.getInstance(context)
        val info = workManager.getWorkInfosForUniqueWork(BackgroundController.WORK_NAME).get(30, TimeUnit.SECONDS)
            .firstOrNull { it.state == WorkInfo.State.ENQUEUED }
            ?: throw AssertionError("Periodische Arbeit nicht eingeplant")
        assertEquals(NetworkType.UNMETERED, info.constraints.requiredNetworkType)
        assertTrue(info.constraints.requiresBatteryNotLow())
        assertFalse(info.constraints.requiresCharging())
        assertEquals(TimeUnit.MINUTES.toMillis(60), info.periodicityInfo?.repeatIntervalMillis)

        delay(3_000)
        assertTrue("Arbeit lief ohne erfüllte Bedingungen", catchUpTasks().none { it.id !in before })

        suspend fun awaitRuns(count: Int) {
            val deadline = System.currentTimeMillis() + 180_000
            while (true) {
                val finished = catchUpTasks().filter { it.id !in before && !it.isActive }
                if (finished.size >= count) return
                if (System.currentTimeMillis() > deadline) throw AssertionError("Nur ${finished.size} von $count Nachhol-Läufen des Workers")
                delay(500)
            }
        }
        driver.setAllConstraintsMet(info.id)
        driver.setPeriodDelayMet(info.id)
        awaitRuns(1)
        driver.setPeriodDelayMet(info.id)
        awaitRuns(2)
        val after = workManager.getWorkInfoById(info.id).get(30, TimeUnit.SECONDS)
        assertNotNull(after)
        assertEquals(WorkInfo.State.ENQUEUED, after!!.state)
        workManager.cancelUniqueWork(BackgroundController.WORK_NAME).result.get(30, TimeUnit.SECONDS)
    }
}
