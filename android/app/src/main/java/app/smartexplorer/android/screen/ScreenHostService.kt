package app.smartexplorer.android.screen

import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.IBinder
import android.util.Log
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import androidx.core.content.IntentCompat
import app.smartexplorer.android.api.ScreenApi
import app.smartexplorer.android.core.CoreException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/**
 * The foreground service of type `mediaProjection` while another device sees this phone's screen
 * (plan 2026-10-17, SCR-M8, C1 Android L1, NR N15): started only by [ScreenConsentActivity] after
 * the system dialog „Bildschirm teilen“ returned its token; it goes to the foreground first and
 * then takes the projection, as Android 14+ demands. Its notification („Bildschirm wird geteilt“,
 * „Beenden“) and the system's status bar chip show the host user that the screen is watched. The
 * projection ends when Android stops it (device locked: `host_locked`), with „Beenden“, or when the
 * viewer leaves. Also answers „Beenden“ and „Ablehnen“ of [ScreenHostNotifier] for sessions without
 * projection. Not exported.
 */
class ScreenHostService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val session = intent?.getLongExtra(EXTRA_SESSION, 0) ?: 0
        when (intent?.action) {
            ACTION_START -> start(intent, session)
            ACTION_STOP -> {
                scope.launch {
                    ScreenHostControl.stopAll(applicationContext)
                    stopIfIdle()
                }
            }
            ACTION_DENY -> {
                ScreenHostNotifier.cancelConsent(this, session)
                scope.launch {
                    try {
                        if (session > 0) ScreenApi.ended(session, "consent_denied")
                    } catch (e: CoreException) {
                        // `not_found`: the viewer left meanwhile.
                    }
                    stopIfIdle()
                }
            }
            else -> stopIfIdle()
        }
        return START_NOT_STICKY
    }

    private fun start(intent: Intent, session: Long) {
        val data = IntentCompat.getParcelableExtra(intent, EXTRA_DATA, Intent::class.java)
        val code = intent.getIntExtra(EXTRA_RESULT_CODE, 0)
        val notification = ScreenHostNotifier.sessionsNotification(applicationContext, ScreenHostControl.sessionsView())
        try {
            ServiceCompat.startForeground(this, ID_FOREGROUND, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
        } catch (e: RuntimeException) {
            Log.w(TAG, "Vordergrunddienst nicht gestartet: ${e.message}")
            scope.launch { ScreenHostControl.projectionFailed(session, "capture_failed") }
            stopIfIdle()
            return
        }
        val manager = getSystemService(MediaProjectionManager::class.java)
        val known = session > 0 || (session == ScreenHostControl.PROBE_SESSION && ScreenHostControl.probe != null)
        val projection: MediaProjection? = if (data != null && known) manager?.getMediaProjection(code, data) else null
        if (projection == null) {
            scope.launch { ScreenHostControl.projectionFailed(session, "consent_denied") }
            stopIfIdle()
            return
        }
        ScreenHostControl.attachProjection(applicationContext, session, projection) { stopIfIdle() }
    }

    private fun stopIfIdle() {
        if (ScreenHostControl.projectionCount() == 0) {
            ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
    }

    override fun onDestroy() {
        ScreenHostControl.releaseProjections()
        super.onDestroy()
    }

    companion object {
        private const val TAG = "SmartExplorerScreen"
        const val ACTION_START = "app.smartexplorer.android.action.SCREEN_START"
        const val ACTION_STOP = "app.smartexplorer.android.action.SCREEN_STOP"
        const val ACTION_DENY = "app.smartexplorer.android.action.SCREEN_DENY"
        const val EXTRA_SESSION = "screenSession"
        private const val EXTRA_RESULT_CODE = "resultCode"
        private const val EXTRA_DATA = "projectionData"
        private const val ID_FOREGROUND = 1417

        /** Starts capturing [session] with the token of the system dialog. */
        fun start(context: Context, session: Long, resultCode: Int, data: Intent) {
            val intent = Intent(context, ScreenHostService::class.java)
                .setAction(ACTION_START)
                .putExtra(EXTRA_SESSION, session)
                .putExtra(EXTRA_RESULT_CODE, resultCode)
                .putExtra(EXTRA_DATA, data)
            ContextCompat.startForegroundService(context, intent)
        }
    }
}
