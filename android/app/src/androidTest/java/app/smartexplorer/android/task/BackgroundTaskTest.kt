package app.smartexplorer.android.task

import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.work.Configuration
import androidx.work.NetworkType
import androidx.work.WorkInfo
import androidx.work.WorkManager
import androidx.work.testing.SynchronousExecutor
import androidx.work.testing.WorkManagerTestInitHelper
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.service.BackgroundController
import app.smartexplorer.android.system.HostMonitor
import java.io.File
import java.math.BigInteger
import java.nio.file.Files
import java.util.Comparator
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.JsonNull
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

    // measure().deferScheduling also depends on background mode/visibility. Read the real
    // window counter, rather than interpreting that Boolean as worker-finally completion.
    private val workerWindows: AtomicInteger by lazy {
        HostMonitor::class.java.getDeclaredField("workerRuns").let { field ->
            field.isAccessible = true
            field.get(HostMonitor) as? AtomicInteger ?: throw AssertionError("HostMonitor.workerRuns is not AtomicInteger")
        }
    }

    private suspend fun waitFor(label: String, timeoutMs: Long, ready: suspend () -> Boolean) {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (!ready()) {
            if (System.currentTimeMillis() >= deadline) throw AssertionError("Timeout: $label")
            delay(200)
        }
    }

    private suspend fun finish(failure: Throwable?, cleanup: suspend () -> Unit) {
        try {
            withContext(NonCancellable) { withTimeout(60_000) { cleanup() } }
        } catch (error: Throwable) {
            if (failure == null) throw error
            failure.addSuppressed(error)
        }
    }

    private suspend fun closeBackgroundWindows() {
        AppPrefs.setBgMode(BackgroundController.MODE_OFF)
        BackgroundController.apply(appContext)
        val manager = WorkManager.getInstance(appContext)
        manager.cancelUniqueWork(BackgroundController.WORK_NAME).result.get(30, TimeUnit.SECONDS)
        Api.call("bg.setSyncEnabled", args("enabled" to false))
        waitFor("background windows and native catch-ups closed", 60_000) {
            val periodic = manager.getWorkInfosForUniqueWork(BackgroundController.WORK_NAME).get(30, TimeUnit.SECONDS)
            val status = Api.obj("bg.status")
            periodic.all { it.state.isFinished } && workerWindows.get() == 0 &&
                !status.bool("syncEnabled") && !status.bool("catchUpRunning") && catchUpTasks().none { it.isActive }
        }
        TaskReport.note("background-idle", "workerWindows=${workerWindows.get()}; periodic and core catch-ups terminal")
    }

    private suspend fun isolatedBackground(work: suspend () -> Unit) {
        val mode = AppPrefs.bgMode.value
        val interval = AppPrefs.bgIntervalMin.value
        val wifiOnly = AppPrefs.bgWifiOnly.value
        val chargingOnly = AppPrefs.bgChargingOnly.value
        val batteryNotLow = AppPrefs.bgBatteryNotLow.value
        val original = Api.obj("bg.status")
        var failure: Throwable? = null
        try {
            HostMonitor.start(appContext)
            closeBackgroundWindows()
            // Host deferral blocks independent new admission, while explicit bg.catchUp
            // still uses the supervisor's open sync/permission gate.
            SyncApi.hostState(HostMonitor.measure(appContext).copy(deferScheduling = true))
            TaskReport.called("sys.hostState", "ok")
            Api.call("bg.resume")
            Api.call("bg.setAutopause", args("battery" to false, "metered" to false))
            work()
        } catch (error: Throwable) { failure = error; throw error }
        finally {
            finish(failure) {
                closeBackgroundWindows()
                Api.call("bg.setAutopause", args("battery" to original.bool("autopauseBattery"), "metered" to original.bool("autopauseMetered")))
                val until = original.textOrNull("pausedUntilMs")?.toLong()
                if (original.bool("paused") && until == null) Api.call("bg.pause", args("seconds" to -1))
                else if (original.bool("paused") && until != null && until > System.currentTimeMillis()) {
                    Api.call("bg.pause", args("seconds" to ((until - System.currentTimeMillis() + 999) / 1000).coerceAtLeast(1)))
                } else Api.call("bg.resume")
                AppPrefs.setBgIntervalMin(interval)
                AppPrefs.setBgWifiOnly(wifiOnly)
                AppPrefs.setBgChargingOnly(chargingOnly)
                AppPrefs.setBgBatteryNotLow(batteryNotLow)
                AppPrefs.setBgMode(mode)
                SyncApi.hostState(HostMonitor.measure(appContext))
                TaskReport.called("sys.hostState", "ok")
                BackgroundController.apply(appContext)
            }
        }
    }

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
        isolatedBackground {
            Api.obj("bg.ensureDaemon")
            Api.call("bg.setSyncEnabled", args("enabled" to true))
            Api.call("bg.resume")
            val root = Fixture.dir(Volumes.primary(), "background", Fixture.unique("catch-up"))
            var jobId: String? = null; var taskId: String? = null; var failure: Throwable? = null
            try {
                val source = File(root, "quelle").apply { assertTrue(mkdirs()) }
                val target = File(root, "ziel").apply { assertTrue(mkdirs()) }
                val targetFile = File(target, "nachholen.txt")
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
                val id = job.text("id"); jobId = id
                assertEquals(5L, job.long("intervalMin"))
                // due::anchor reads these exact native hexadecimal creation nanoseconds.
                // Keep the real five-minute interval instead of forging an old id/state.
                val createdMs = BigInteger(id, 16).divide(BigInteger.valueOf(1_000_000_000L)).longValueExact() * 1000
                val dueMs = createdMs + TimeUnit.MINUTES.toMillis(job.long("intervalMin"))
                TaskReport.note("background-due", "job=$id createdMs=$createdMs dueMs=$dueMs")
                withTimeout(300_000) {
                    while (true) {
                        val remaining = dueMs - System.currentTimeMillis()
                        if (remaining <= 0) break
                        delay(minOf(500L, remaining))
                    }
                }
                val untouched = SyncJobs.byId(id).field("state").obj()
                assertEquals("Independent scheduling ran fixture job: $untouched", JsonNull, untouched.field("lastAttemptMs"))
                assertEquals(JsonNull, untouched.field("lastSuccessMs"))
                assertEquals(JsonNull, untouched.field("running"))
                assertEquals("Background worker window still open", 0, workerWindows.get())
                SyncApi.hostState(HostMonitor.measure(appContext).copy(deferScheduling = true))
                TaskReport.called("sys.hostState", "ok")
                val requestedMs = System.currentTimeMillis() / 1000 * 1000
                val started = Api.start("bg.catchUp"); taskId = started
                val run = Api.await(started, timeoutMs = 300_000)
                assertEquals("catchup", run.kind)
                assertEquals("$run", "done", run.state)
                val result = run.resultObj()
                val admitted = result.int("admitted")
                val skipped = result.objects("skipped")
                val ownSkip = skipped.any { it.text("jobId") == id && it.text("jobName") == name && it.text("reason").isNotBlank() }
                TaskReport.note("bg.catchUp", "job=$id state=${run.state} message=${run.message} result=$result")
                assertTrue("Eigener Job weder zugelassen noch mit Grund übersprungen: $result", admitted >= 1 || ownSkip)
                val deadline = System.currentTimeMillis() + 120_000
                while (!targetFile.isFile && System.currentTimeMillis() < deadline) delay(500)
                assertTrue("Fälliger Job lief nicht", targetFile.isFile)
                assertEquals("fällig", targetFile.readText(Charsets.UTF_8))
                val state = SyncJobs.byId(id).field("state").obj()
                assertTrue("Own success predates catch-up: $state", state.long("lastSuccessMs") >= requestedMs)
                assertEquals("catch_up", state.text("lastCause"))
                assertEquals(0, state.int("consecutiveFailures"))
                assertEquals(JsonNull, state.field("lastError"))
                assertEquals(JsonNull, state.field("running"))
                TaskReport.note("background-own-job", "job=$id state=$state bytes=${targetFile.length()}")
                assertNotNull(Api.obj("bg.status").textOrNull("lastCatchUpMs"))
            } catch (error: Throwable) { failure = error; throw error }
            finally {
                finish(failure) {
                    taskId?.let { id ->
                        if (Api.task(id).isActive) Api.call("task.cancel", args("id" to id))
                        Api.await(id, 30_000)
                    }
                    jobId?.let { id ->
                        waitFor("own catch-up job released", 30_000) { SyncJobs.byId(id).field("state").obj().field("running") == JsonNull }
                        Api.call("sync.delete", args("id" to id))
                    }
                    if (root.exists()) Files.walk(root.toPath()).use { paths ->
                        paths.sorted(Comparator.reverseOrder()).forEach { Files.deleteIfExists(it) }
                    }
                }
            }
        }
    }

    @Test
    fun periodicWorkerRunsCatchUpWhenConstraintsAndPeriodAreMet() = coreTest {
        isolatedBackground {
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
            waitFor("worker sync flag enabled", 60_000) { Api.obj("bg.status").bool("syncEnabled") }

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
                    if (finished.size >= count) {
                        finished.forEach { assertEquals("Worker catch-up did not succeed: $it", "done", it.state) }
                        return
                    }
                    if (System.currentTimeMillis() > deadline) throw AssertionError("Nur ${finished.size} von $count Nachhol-Läufen des Workers")
                    delay(500)
                }
            }
            // The worker returns only after its catch-up task ended; a period signal while it still
            // runs is ignored, so the next one waits until the work is enqueued again.
            suspend fun awaitEnqueued() {
                val deadline = System.currentTimeMillis() + 60_000
                while (workManager.getWorkInfoById(info.id).get(30, TimeUnit.SECONDS)?.state != WorkInfo.State.ENQUEUED || workerWindows.get() != 0) {
                    if (System.currentTimeMillis() > deadline) throw AssertionError("Periodische Arbeit nicht wieder eingeplant")
                    delay(200)
                }
            }
            driver.setAllConstraintsMet(info.id)
            driver.setPeriodDelayMet(info.id)
            awaitRuns(1)
            awaitEnqueued()
            // The TestScheduler starts each period with fresh state: constraints unmet again.
            driver.setAllConstraintsMet(info.id)
            driver.setPeriodDelayMet(info.id)
            awaitRuns(2)
            awaitEnqueued()
            val after = workManager.getWorkInfoById(info.id).get(30, TimeUnit.SECONDS)
            assertNotNull(after)
            assertEquals(WorkInfo.State.ENQUEUED, after!!.state)
            assertEquals("Worker finally left a window open", 0, workerWindows.get())
            TaskReport.note("periodic-worker-terminal", "work=${info.id} state=${after.state}; workerWindows=${workerWindows.get()}")
            workManager.cancelUniqueWork(BackgroundController.WORK_NAME).result.get(30, TimeUnit.SECONDS)
        }
    }
}
