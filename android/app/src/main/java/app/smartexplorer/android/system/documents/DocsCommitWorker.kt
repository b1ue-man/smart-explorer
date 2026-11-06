package app.smartexplorer.android.system.documents

import android.app.Application
import android.app.Notification
import android.content.Context
import android.content.pm.ServiceInfo
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingWorkPolicy
import androidx.work.ForegroundInfo
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.OutOfQuotaPolicy
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import app.smartexplorer.android.api.DocsPending
import app.smartexplorer.android.R
import app.smartexplorer.android.api.DocsApi
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.system.Notifications
import java.util.concurrent.TimeUnit

/**
 * Uploads what other apps saved through the documents provider (DRV-M8, NR N14): the core
 * publishes right after `onRelease`; this expedited job keeps the process alive meanwhile and,
 * after a process end, retries what the journal still holds (`docs.retryPending`). The provider
 * process has no exemption to start a foreground service, so the job brings its own
 * [getForegroundInfo] (dataSync, required for expedited work below Android 12). After
 * [MAX_ATTEMPTS] failed runs it tells the person; the changes stay queued. The waiting state it
 * gave up on is remembered: a later process start retries that same state once
 * ([enqueueAtStart]) rather than with the full series and a second notice.
 */
class DocsCommitWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun getForegroundInfo(): ForegroundInfo =
        ForegroundInfo(ID_RUNNING, running(applicationContext), ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)

    override suspend fun doWork(): Result {
        val app = applicationContext as Application
        AppPrefs.init(app)
        Core.start(app)
        val pending = try {
            DocsApi.retryPending()
        } catch (e: CoreException) {
            Log.w(TAG, "docs.retryPending: ${e.kind}")
            return retryOrGiveUp(null)
        }
        if (pending.isClean) {
            Notifications.cancel(applicationContext, ID_FAILED)
            givenUp(applicationContext, null)
            return Result.success()
        }
        return retryOrGiveUp(pending)
    }

    private fun retryOrGiveUp(pending: DocsPending?): Result {
        // A single attempt of a state already given up on ends quietly: the person was told.
        if (inputData.getBoolean(KEY_SINGLE, false)) return Result.failure()
        if (runAttemptCount + 1 < MAX_ATTEMPTS) return Result.retry()
        givenUp(applicationContext, fingerprintOf(pending))
        failed(applicationContext)
        return Result.failure()
    }

    companion object {
        private const val TAG = "SmartExplorerDocs"
        private const val WORK_NAME = "docs-commit"
        const val ID_RUNNING = 1500
        const val ID_FAILED = 1501
        const val MAX_ATTEMPTS = 6
        private const val KEY_SINGLE = "single-attempt"
        private const val PREFS = "docs-commit"
        private const val GIVEN_UP = "given-up"

        /** Queues a run after the current one (each run waits for the uploads in flight). */
        fun enqueue(context: Context) = enqueue(context, single = false)

        /**
         * The process start found [pending] (`null`: `docs.pending` failed). The full series, unless
         * the worker already gave up on this same state (or the state is unknown): then one attempt.
         */
        fun enqueueAtStart(context: Context, pending: DocsPending?) {
            val known = pending != null && context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                .getString(GIVEN_UP, null) != fingerprintOf(pending)
            enqueue(context, single = !known)
        }

        private fun fingerprintOf(pending: DocsPending?): String = pending?.fingerprint ?: "?"

        /** Remembers the waiting state the series gave up on (`null`: none, after a clean run). */
        private fun givenUp(context: Context, fingerprint: String?) {
            val editor = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            if (fingerprint == null) editor.remove(GIVEN_UP) else editor.putString(GIVEN_UP, fingerprint)
            editor.apply()
        }

        private fun enqueue(context: Context, single: Boolean) {
            val request = OneTimeWorkRequestBuilder<DocsCommitWorker>()
                .setInputData(workDataOf(KEY_SINGLE to single))
                .setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST)
                .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS)
                .build()
            WorkManager.getInstance(context).enqueueUniqueWork(WORK_NAME, ExistingWorkPolicy.APPEND_OR_REPLACE, request)
        }

        fun running(context: Context): Notification {
            Notifications.ensureChannels(context)
            return NotificationCompat.Builder(context, Notifications.CHANNEL_TRANSFERS)
                .setSmallIcon(R.drawable.ic_upload)
                .setContentTitle(context.getString(R.string.docs_commit_title))
                .setContentText(context.getString(R.string.docs_commit_text))
                .setOngoing(true)
                .setSilent(true)
                .build()
        }

        /** Saved changes of other apps could not be uploaded (also after a failed release). */
        fun failed(context: Context) {
            Notifications.ensureChannels(context)
            val notification = NotificationCompat.Builder(context, Notifications.CHANNEL_TRANSFERS)
                .setSmallIcon(R.drawable.ic_warning)
                .setContentTitle(context.getString(R.string.docs_commit_failed_title))
                .setContentText(context.getString(R.string.docs_commit_failed_text))
                .setStyle(NotificationCompat.BigTextStyle().bigText(context.getString(R.string.docs_commit_failed_text)))
                .setAutoCancel(true)
                .build()
            Notifications.notify(context, ID_FAILED, notification)
        }
    }
}
