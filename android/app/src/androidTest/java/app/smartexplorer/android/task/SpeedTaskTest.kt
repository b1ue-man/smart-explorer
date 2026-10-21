package app.smartexplorer.android.task

import android.net.Uri
import android.os.ParcelFileDescriptor
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.File
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Measurement cases of the stage `speed` on the emulator (U55, M-3 to M-6 "Android"): the core's local copy
 * against the emulator's own `dd` (`-e speed_dd_write_bps/speed_dd_read_bps`, measured by the host script with
 * `adb shell dd`, fixture `speed-dd`) and against `toybox cp -r`. The emulator is no measure of real devices
 * (A-S9): the requirement holds against its own tools only. Every case logs one JSON line with the tag
 * `SpeedTask`; the host script adds it to `speed_report.json`.
 */
@RunWith(AndroidJUnit4::class)
class SpeedTaskTest {
    private val tag = "SpeedTask"
    private val mib = 1024L * 1024L

    private suspend fun seconds(block: suspend () -> Unit): Double {
        val started = System.nanoTime()
        block()
        return (System.nanoTime() - started) / 1e9
    }

    private fun report(
        name: String,
        unit: String,
        rate: Double,
        capacity: Double,
        required: Double,
        requirement: String,
        tool: String,
        sizeBytes: Long,
        files: Long,
        notes: List<String>,
    ) {
        val line = JSONObject()
            .put("name", name).put("status", "measured").put("way", "android").put("direction", "copy")
            .put("profile", "-").put("size_bytes", sizeBytes).put("files", files).put("unit", unit)
            .put("se_rate", rate).put("capacity", capacity).put("share", if (capacity > 0) rate / capacity else 0.0)
            .put("required", required).put("requirement", requirement).put("reference_tool", tool)
            .put("reference_rate", capacity).put("limiter", "").put("notes", JSONArray(notes))
            .put("skipped", JSONArray())
        Log.i(tag, line.toString())
        TaskReport.note("speed", line.toString())
    }

    /** Which sensor files an app may read (A-S6: the plan assumes none of the first three). */
    private fun sensors(): String =
        listOf("/proc/stat", "/proc/diskstats", "/proc/net/route", "/sys/class/net", "/proc/meminfo", "/proc/self/stat")
            .joinToString(", ") { path -> "$path=" + if (runCatching { File(path).let { it.canRead() && (it.isDirectory || it.readText().isNotEmpty()) } }.getOrDefault(false)) "lesbar" else "nicht lesbar" }

    @Test
    fun speed_task_android_local_large() = coreTest(30 * 60_000L) {
        val volume = Volumes.primary()
        val source = Fixture.big(File(Fixture.dir(volume, "speed", "large-src"), "large.bin"), 1024, 20261021L)
        val target = Fixture.dir(volume, "speed", "large-dst")
        val write = TaskArgs.get("speed_dd_write_bps").toDouble()
        val read = TaskArgs.get("speed_dd_read_bps").toDouble()
        val capacity = minOf(write, read)
        var rate = 0.0
        var firstRate = 0.0
        // A shortfall is repeated once (M-1: shared runners are noisy); both values reach the report.
        for (attempt in 0 until 2) {
            File(target, "large.bin").delete()
            val elapsed = seconds { Api.copy(listOf(source.absolutePath), target.absolutePath) }
            assertEquals(source.length(), File(target, "large.bin").length())
            rate = source.length() / elapsed
            if (attempt == 0) firstRate = rate
            if (rate >= 0.85 * capacity) break
        }
        report(
            "speed_task_android_local_large", "B/s", rate, capacity, 0.85, "≥ 0,85 × dd (min aus Schreiben und Lesen)",
            "adb shell dd bs=1M (${TaskArgs.get("speed_dd_note")})", source.length(), 1,
            listOf("erster Lauf ${"%.1f".format(firstRate / 1e6)} MB/s", "Sensoren: ${sensors()}", "Emulator, kein Maß für Geräte (A-S9)"),
        )
        assertTrue(
            "speed_task_android_local_large: ${"%.1f".format(rate / 1e6)} MB/s < 0,85 × ${"%.1f".format(capacity / 1e6)} MB/s (dd aus adb shell dd)",
            rate >= 0.85 * capacity,
        )
    }

    @Test
    fun speed_task_android_local_small() = coreTest(30 * 60_000L) {
        val volume = Volumes.primary()
        val source = Fixture.dir(volume, "speed", "small-src", "tree")
        val files = 5000
        for (index in 0 until files) {
            Fixture.bytes(File(source, "d%03d/f%05d.bin".format(index % 50, index)), 4096, index + 1)
        }
        val reference = Fixture.dir(volume, "speed", "small-ref")
        val toybox = seconds {
            val process = ProcessBuilder("/system/bin/toybox", "cp", "-r", source.absolutePath, reference.absolutePath)
                .redirectErrorStream(true).start()
            val output = process.inputStream.bufferedReader().readText()
            assertEquals("toybox cp -r: $output", 0, process.waitFor())
        }
        val toyboxRate = files / toybox
        var rate = 0.0
        for (attempt in 0 until 2) {
            val target = Fixture.dir(volume, "speed", "small-dst")
            val elapsed = seconds { Api.copy(listOf(source.absolutePath), target.absolutePath) }
            rate = files / elapsed
            if (rate >= toyboxRate) break
        }
        report(
            "speed_task_android_local_small", "files/s", rate, toyboxRate, 1.0, "≥ 1,0 × toybox cp -r", "toybox cp -r",
            files * 4096L, files.toLong(), listOf("Sensoren: ${sensors()}", "Emulator, kein Maß für Geräte (A-S9)"),
        )
        assertTrue("speed_task_android_local_small: ${"%.1f".format(rate)} < 1,0 × ${"%.1f".format(toyboxRate)} files/s (toybox cp -r)", rate >= toyboxRate)
    }

    /** Report only: 512 MiB read through `ContentResolver.openFileDescriptor` and through the file path. */
    @Test
    fun speed_task_android_content_vs_file_read() = coreTest {
        val volume = Volumes.primary()
        val file = Fixture.big(File(Fixture.dir(volume, "speed", "read"), "read.bin"), 512, 7L)
        val buffer = ByteArray(1024 * 1024)
        val viaFile = seconds { file.inputStream().use { input -> while (input.read(buffer) >= 0) Unit } }
        val viaContent = seconds {
            val descriptor: ParcelFileDescriptor = appContext.contentResolver.openFileDescriptor(Uri.fromFile(file), "r")
                ?: throw AssertionError("openFileDescriptor lieferte null für ${file.path}")
            ParcelFileDescriptor.AutoCloseInputStream(descriptor).use { input -> while (input.read(buffer) >= 0) Unit }
        }
        val size = file.length().toDouble()
        report(
            "speed_task_android_content_vs_file_read", "B/s", size / viaContent, size / viaFile, 0.0,
            "nur Bericht: ContentResolver gegen Dateipfad", "Dateipfad (FileInputStream)", file.length(), 1,
            listOf("Dateipfad ${"%.1f".format(size / viaFile / 1e6)} MB/s, ContentResolver ${"%.1f".format(size / viaContent / 1e6)} MB/s", "Seitencache nicht geleert"),
        )
    }
}
