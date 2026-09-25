package app.smartexplorer.android.task

import kotlinx.serialization.json.JsonObject

/** Sync jobs built from the core's own defaults (`sync.options.defaults`), never from guessed enums. */
object SyncJobs {
    suspend fun defaults(): JsonObject = Api.obj("sync.options").field("defaults").obj()

    /** A job JSON: the defaults with [overrides] applied (values as in [args]). */
    suspend fun job(vararg overrides: Pair<String, Any?>): JsonObject = JsonObject(defaults() + args(*overrides))

    /** Saves a new job and returns the stored job (with its id). */
    suspend fun save(vararg overrides: Pair<String, Any?>): JsonObject =
        Api.obj("sync.save", args("job" to job("id" to "", *overrides)))

    suspend fun byId(id: String): JsonObject =
        Api.objects("sync.jobs").firstOrNull { it.text("id") == id } ?: throw AssertionError("Job $id fehlt in sync.jobs")
}
