package app.smartexplorer.android.screen

import android.app.ActivityOptions
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.unit.dp
import androidx.lifecycle.lifecycleScope
import app.smartexplorer.android.api.AccessApproval
import app.smartexplorer.android.api.DECISION_DENY
import app.smartexplorer.android.api.ScreenApi
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.ScreenHostStart
import app.smartexplorer.android.service.AccessApprovalNotifier
import app.smartexplorer.android.ui.theme.SmartExplorerTheme
import kotlinx.coroutines.flow.filterIsInstance
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/**
 * The se consent activity of this phone as screen host (plan 2026-10-17, SCR-M8, C7, E1 point 7).
 * Two ways in, both only through intents of this app (not exported):
 *
 * - from the question of another device (S1 `access_approval`, „Diesmal erlauben“ / „Immer
 *   erlauben“ in the notification or the dialog): it answers the question, waits for the session
 *   the core then starts for that device and asks the system dialog „Bildschirm teilen“;
 * - from a session the core started without a question (right on „Erlauben“), or a preview that
 *   becomes a full stream: it asks the system dialog for that session.
 *
 * Only after the system dialog's „Starten“ does [ScreenHostService] capture; a cancelled dialog
 * ends the session with `consent_denied`. Every screen session thus needs both the S1 right and
 * the system dialog (`consent_each_session`). The activity never answers silently.
 */
class ScreenConsentActivity : ComponentActivity() {
    private var session: Long = 0
    private var waiting by mutableStateOf(false)

    private val capture = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        val data = result.data
        if (result.resultCode == RESULT_OK && data != null && session > 0) {
            ScreenHostService.start(this, session, result.resultCode, data)
        } else if (session > 0) {
            val ended = session
            lifecycleScope.launch { endQuietly(ended, "consent_denied") }
        }
        finish()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            SmartExplorerTheme {
                if (waiting) {
                    AlertDialog(
                        onDismissRequest = {},
                        title = { Text("Bildschirmfreigabe wird vorbereitet") },
                        text = {
                            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                                CircularProgressIndicator()
                                Text("Danach fragt Android, ob der Bildschirm geteilt werden darf.")
                            }
                        },
                        confirmButton = {},
                        dismissButton = { TextButton(onClick = { finish() }) { Text("Abbrechen") } },
                    )
                }
            }
        }
        if (savedInstanceState != null) {
            session = savedInstanceState.getLong(EXTRA_SESSION)
            return
        }
        val approvalId = intent.getStringExtra(EXTRA_APPROVAL_ID)
        val decision = intent.getStringExtra(EXTRA_DECISION)
        val started = intent.getLongExtra(EXTRA_SESSION, 0)
        when {
            started > 0 -> askSystem(started)
            approvalId != null && decision != null && decision != DECISION_DENY ->
                answerThenAsk(approvalId, decision, intent.getStringExtra(EXTRA_DEVICE_ID).orEmpty())
            else -> finish()
        }
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        outState.putLong(EXTRA_SESSION, session)
    }

    /** The S1 answer first, then the system dialog for the session the core starts for [deviceId]. */
    private fun answerThenAsk(approvalId: String, decision: String, deviceId: String) {
        waiting = true
        lifecycleScope.launch {
            val waiter = launch {
                val start = withTimeoutOrNull(START_WAIT_MS) {
                    ScreenHostControl.claimPending(deviceId) ?: Core.events.filterIsInstance<ScreenHostStart>()
                        .first { !it.still && it.viewerDeviceId == deviceId && ScreenHostControl.claim(it.session) }
                }
                waiting = false
                if (start == null) finish() else askSystem(start.session)
            }
            AccessApprovalNotifier.decideScreen(approvalId, decision)
            waiter.join()
        }
    }

    private fun askSystem(id: Long) {
        session = id
        ScreenHostNotifier.cancelConsent(this, id)
        val manager = getSystemService(MediaProjectionManager::class.java)
        if (manager == null) {
            lifecycleScope.launch { endQuietly(id, "capture_failed") }
            finish()
            return
        }
        capture.launch(manager.createScreenCaptureIntent())
    }

    private suspend fun endQuietly(id: Long, reason: String) {
        try {
            ScreenApi.ended(id, reason)
        } catch (e: CoreException) {
            // `not_found`: the viewer left meanwhile.
        }
    }

    companion object {
        const val EXTRA_SESSION = "screenSession"
        const val EXTRA_APPROVAL_ID = "approvalId"
        const val EXTRA_DECISION = "decision"
        const val EXTRA_DEVICE_ID = "deviceId"
        private const val START_WAIT_MS = 30_000L

        /** For a session the core started ([ScreenHostStart]). */
        fun sessionIntent(context: Context, start: ScreenHostStart): Intent =
            Intent(context, ScreenConsentActivity::class.java)
                .setData(android.net.Uri.parse("se-screen://session/${start.session}"))
                .putExtra(EXTRA_SESSION, start.session)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

        /** For an allowing answer ([decision] `once` or `always`) to the screen question [approval]. */
        fun approvalIntent(context: Context, approval: AccessApproval, decision: String): Intent =
            Intent(context, ScreenConsentActivity::class.java)
                .setData(android.net.Uri.parse("se-screen://approval/${approval.id}/$decision"))
                .putExtra(EXTRA_APPROVAL_ID, approval.id)
                .putExtra(EXTRA_DECISION, decision)
                .putExtra(EXTRA_DEVICE_ID, approval.deviceId)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

        /**
         * [intent] as an immutable activity PendingIntent; on API 34+ its creator grants the start
         * from the background, so a tap in the notification opens the activity (NR N15, target 36).
         */
        fun pendingIntent(context: Context, requestCode: Int, intent: Intent): PendingIntent {
            val options = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                ActivityOptions.makeBasic()
                    .setPendingIntentCreatorBackgroundActivityStartMode(ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOWED)
                    .toBundle()
            } else {
                null
            }
            return PendingIntent.getActivity(
                context,
                requestCode,
                intent,
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
                options,
            )
        }
    }
}
