package app.smartexplorer.android.service

import android.app.Activity
import android.app.Application
import android.app.Notification
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.util.Log
import androidx.annotation.DrawableRes
import androidx.core.app.NotificationCompat
import app.smartexplorer.android.R
import app.smartexplorer.android.api.AccessApproval
import app.smartexplorer.android.api.DECISION_ALWAYS
import app.smartexplorer.android.api.DECISION_DENY
import app.smartexplorer.android.api.DECISION_ONCE
import app.smartexplorer.android.api.ShareAccessApi
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.screen.ScreenConsentActivity
import app.smartexplorer.android.screen.ScreenHostControl
import app.smartexplorer.android.system.Notifications
import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.MainTab
import app.smartexplorer.android.ui.NavRequest
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.share.AccessApprovalActivity
import app.smartexplorer.android.ui.share.approvalLines
import app.smartexplorer.android.ui.share.approvalTitle
import java.lang.ref.WeakReference
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** How this phone shows a question of another device (plan 2026-10-17, C5). */
internal enum class ApprovalDelivery {
    /** As a dialog in the visible app ([AccessApprovalActivity]). */
    InApp,

    /** As a notification of the channel `access_approval`. */
    Notification,

    /** Nowhere: the app reports it and the other device gets `approval_unavailable`. */
    Undeliverable,
}

/**
 * The rule of C5: while the app is [visible] the question is a dialog in the app; otherwise a
 * notification. A visible app keeps the question open even when it can show neither (the page
 * „Teilen“ lists it); only a hidden app that cannot post a notification gives up on it.
 */
internal fun approvalDelivery(visible: Boolean, showInApp: () -> Boolean, post: () -> Boolean): ApprovalDelivery = when {
    visible && showInApp() -> ApprovalDelivery.InApp
    post() -> ApprovalDelivery.Notification
    visible -> ApprovalDelivery.InApp
    else -> ApprovalDelivery.Undeliverable
}

/** [approvalDelivery] for [approval]; an undeliverable question is reported through [report]. */
internal suspend fun deliverApproval(
    approval: AccessApproval,
    visible: Boolean,
    showInApp: () -> Boolean,
    post: () -> Boolean,
    report: suspend (String) -> Unit,
): ApprovalDelivery {
    val delivery = approvalDelivery(visible, showInApp, post)
    if (delivery == ApprovalDelivery.Undeliverable) report(approval.id)
    return delivery
}

/**
 * The notification of [approval]: heads-up on the channel `access_approval`; tapping it opens the
 * app, where the question shows as a dialog. Exec questions carry „Diesmal erlauben“, „Immer
 * erlauben“ and „Ablehnen“ as broadcasts to the non-exported [AccessApprovalReceiver]; both allowing
 * actions need an unlocked device (API 31+, `setAuthenticationRequired`). Android 11 cannot demand
 * that for an action, so there they are left out and only the dialog in the app allows. The allowing
 * actions of a screen question are activity intents to [ScreenConsentActivity] (SCR-M8, W1): it
 * answers the question and then asks the system dialog „Bildschirm teilen“, so allowing the screen
 * never happens silently; on API 34+ their creator grants the start from the background. Swiping it
 * away answers „Ablehnen“. On the lock screen only a public version without device, command or
 * actions shows.
 */
internal fun approvalNotification(context: Context, approval: AccessApproval, id: Int): Notification {
    val open = shareTabPendingIntent(context, id)
    val lines = approvalLines(approval)
    val hidden = NotificationCompat.Builder(context, Notifications.CHANNEL_ACCESS_APPROVAL)
        .setSmallIcon(R.drawable.ic_lock)
        .setContentTitle("Ein Gerät wartet auf deine Bestätigung")
        .setContentText("Entsperren, um die Rückfrage zu sehen")
        .build()
    val builder = NotificationCompat.Builder(context, Notifications.CHANNEL_ACCESS_APPROVAL)
        .setSmallIcon(R.drawable.ic_lock)
        .setContentTitle(approvalTitle(approval))
        .setContentText(lines.drop(1).firstOrNull { it.startsWith("Programm") || it.startsWith("Shell") } ?: lines.firstOrNull().orEmpty())
        .setStyle(NotificationCompat.BigTextStyle().bigText(lines.joinToString("\n")))
        .setPriority(NotificationCompat.PRIORITY_HIGH)
        .setOnlyAlertOnce(true)
        .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
        .setPublicVersion(hidden)
        .setContentIntent(open)
        .setDeleteIntent(answerIntent(context, approval.id, DECISION_DENY))
    if (approval.screen && approval.canAllow) {
        builder.addAction(screenAction(context, approval, DECISION_ONCE, id, "Diesmal erlauben"))
        builder.addAction(screenAction(context, approval, DECISION_ALWAYS, id, "Immer erlauben"))
    } else if (!approval.contextual && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        builder.addAction(answerAction(context, approval.id, DECISION_ONCE, R.drawable.ic_check, "Diesmal erlauben", unlocked = true))
        builder.addAction(answerAction(context, approval.id, DECISION_ALWAYS, R.drawable.ic_check, "Immer erlauben", unlocked = true))
    }
    builder.addAction(answerAction(context, approval.id, DECISION_DENY, R.drawable.ic_close, "Ablehnen", unlocked = false))
    return builder.build()
}

/**
 * An allowing answer to the screen question [approval]: an activity intent to
 * [ScreenConsentActivity] (never a broadcast), which needs an unlocked device.
 */
internal fun screenAction(context: Context, approval: AccessApproval, decision: String, id: Int, label: String): NotificationCompat.Action {
    val code = id * 4 + if (decision == DECISION_ALWAYS) 1 else 0
    val intent = ScreenConsentActivity.pendingIntent(context, code, ScreenConsentActivity.approvalIntent(context, approval, decision))
    return NotificationCompat.Action.Builder(R.drawable.ic_check, label, intent)
        .setAuthenticationRequired(true)
        .build()
}

/** One answer that names [approvalId]: its own data URI keeps it apart from every other question. */
private fun answerIntent(context: Context, approvalId: String, decision: String): PendingIntent =
    PendingIntent.getBroadcast(
        context,
        0,
        Intent(context, AccessApprovalReceiver::class.java)
            .setAction(ACTION_ACCESS_APPROVAL)
            .setData(approvalUri(approvalId, decision)),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

private fun answerAction(
    context: Context,
    approvalId: String,
    decision: String,
    @DrawableRes icon: Int,
    label: String,
    unlocked: Boolean,
): NotificationCompat.Action =
    NotificationCompat.Action.Builder(icon, label, answerIntent(context, approvalId, decision))
        .setAuthenticationRequired(unlocked)
        .build()

/** The lowest notification id of the question range not in [used]; `null` when all are taken. */
internal fun freeApprovalNotificationId(used: Collection<Int>): Int? =
    (Notifications.ID_APPROVAL_FIRST until Notifications.ID_APPROVAL_FIRST + Notifications.APPROVAL_ID_COUNT).firstOrNull { it !in used }

/** Opens the app on its Share tab; [id] keeps the PendingIntents of different notifications apart. */
private fun shareTabPendingIntent(context: Context, id: Int): PendingIntent =
    PendingIntent.getActivity(
        context,
        id,
        AppNav.intentFor(context, NavRequest.SelectTab(MainTab.Share)),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

/** The open questions of one requesting device (E28, M32); [key] is the device id. */
internal data class DeviceApprovals(
    val key: String,
    val deviceName: String,
    val approvals: List<AccessApproval>,
) {
    /** The number of open questions of the device, which its one notification announces. */
    val count: Int
        get() = approvals.size
}

/** The group of a question: its requesting device; a question without a device id forms a group of its own. */
internal fun deviceGroupKey(approval: AccessApproval): String = approval.deviceId.ifBlank { approval.id }

/** The open questions grouped by requesting device, in the order each device's first question came. */
internal fun deviceApprovalGroups(list: List<AccessApproval>): List<DeviceApprovals> =
    list.groupBy { deviceGroupKey(it) }.map { (key, items) -> DeviceApprovals(key, items.first().deviceName, items) }

/**
 * The notification of a device with two or more open questions (E28, M32): it names their number and
 * the device. Tapping it opens the app, where the dialog answers each question on its own. It has no
 * answer action, since one would answer one question of several, and swiping it away answers nothing:
 * the questions stay open, and the notification returns when their number changes.
 */
internal fun deviceApprovalsNotification(context: Context, device: DeviceApprovals, id: Int): Notification {
    val name = device.deviceName.ifBlank { "einem Gerät" }
    val hidden = NotificationCompat.Builder(context, Notifications.CHANNEL_ACCESS_APPROVAL)
        .setSmallIcon(R.drawable.ic_lock)
        .setContentTitle("${device.count} Rückfragen warten auf deine Bestätigung")
        .setContentText("Entsperren, um die Rückfragen zu sehen")
        .build()
    return NotificationCompat.Builder(context, Notifications.CHANNEL_ACCESS_APPROVAL)
        .setSmallIcon(R.drawable.ic_lock)
        .setContentTitle("${device.count} Rückfragen von $name")
        .setContentText("Öffnen, um jede Rückfrage einzeln zu beantworten")
        .setPriority(NotificationCompat.PRIORITY_HIGH)
        .setOnlyAlertOnce(true)
        .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
        .setPublicVersion(hidden)
        .setContentIntent(shareTabPendingIntent(context, id))
        .build()
}

/**
 * Questions of other devices that wait for this phone's user (plan 2026-10-17, PRM-M10). Woken by
 * [ExecHostNotifier] on every `share` event (the core sends one when a question opens or ends) and
 * by the app's visibility; reads `share.accessApprovals` and shows each question as a dialog in the
 * visible app or as a notification ([approvalDelivery]). The notifications of one requesting device
 * are one notification that names the number of its open questions ([DeviceApprovals], E28, M32).
 * When the app is hidden and cannot post a notification (`POST_NOTIFICATIONS` missing or
 * notifications off) it reports `share.reportApprovalUndeliverable`, so the other device is refused
 * with `approval_unavailable` instead of waiting. Every answer names one question; „Immer erlauben“
 * and „Ablehnen“ also answer the open questions equal to it (E28). Runs for the whole process once
 * [start]ed; all state is touched on the main thread.
 */
object AccessApprovalNotifier {
    private const val TAG = "SmartExplorerAccess"
    private const val RECHECK_MS = 5_000L

    /** A rotation stops and starts the activity: hidden only after this long. */
    private const val HIDDEN_AFTER_MS = 1_500L

    /** A started dialog counts as shown while it comes up. */
    private const val PROMPT_START_MS = 5_000L

    private val started = AtomicBoolean(false)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val refreshes = MutableStateFlow(0L)
    private val pendingFlow = MutableStateFlow<List<AccessApproval>>(emptyList())

    /** The open questions, oldest first, without those just answered. */
    val pending: StateFlow<List<AccessApproval>> = pendingFlow.asStateFlow()

    /** The notification of each requesting device with open questions. */
    private val notified = LinkedHashMap<String, Notice>()

    /** A device's notification: its id and the number of open questions it announced. */
    private class Notice(val id: Int, val count: Int)
    private val reported = HashSet<String>()
    private val answering = HashSet<String>()
    private var startedActivities = 0
    private var startedPrompts = 0
    private var hiddenSince: Long? = SystemClock.elapsedRealtime()
    private var promptLaunchedAt: Long? = null
    private var resumed = WeakReference<Activity>(null)

    fun start(context: Context) {
        if (!started.compareAndSet(false, true)) return
        val app = context.applicationContext as Application
        app.registerActivityLifecycleCallbacks(Visibility)
        ScreenHostControl.start(app)
        // A StateFlow is conflated: a burst of events leads to at most one more load.
        scope.launch { refreshes.collect { refresh(app) } }
        scope.launch {
            while (true) {
                delay(RECHECK_MS)
                if (pendingFlow.value.isNotEmpty() || notified.isNotEmpty()) requestRefresh()
            }
        }
    }

    fun requestRefresh() {
        refreshes.update { it + 1 }
    }

    /** The visible activity of the app, `null` while the app is hidden. */
    internal fun foregroundActivity(): Activity? = resumed.get()

    /**
     * Answers [approvalId] with [decision] (`once`, `always`, `deny`; `always` and `deny` also answer
     * the open questions equal to it, E28); [done] ends the broadcast of a notification action. A
     * question that is no longer open stays unanswered. A
     * screen question is allowed only through [ScreenConsentActivity] (system dialog afterwards):
     * from the visible app it opens there; without a visible app nothing is allowed.
     */
    internal fun decide(approvalId: String, decision: String, done: () -> Unit = {}) {
        val current = pendingFlow.value.firstOrNull { it.id == approvalId }
        if (decision != DECISION_DENY && (current == null || !current.canAllow)) {
            Snackbars.show("Vollständigen Zielauftrag zuerst laden und prüfen. Erlauben ist gesperrt.")
            requestRefresh()
            done()
            return
        }
        val screen = pendingFlow.value.firstOrNull { it.id == approvalId }?.takeIf { it.screen }
        if (screen != null && decision != DECISION_DENY) {
            val activity = resumed.get()
            try {
                activity?.startActivity(ScreenConsentActivity.approvalIntent(activity, screen, decision))
                    ?: Log.w(TAG, "Bildschirmfreigabe nur aus der sichtbaren App")
            } catch (e: RuntimeException) {
                Log.w(TAG, "Zustimmung nicht geöffnet", e)
            }
            done()
            return
        }
        answer(approvalId, decision, done)
    }

    /** The answer of [ScreenConsentActivity] to a screen question (it asks the system dialog next). */
    internal suspend fun decideScreen(approvalId: String, decision: String) {
        answering += approvalId
        pendingFlow.update { list -> list.filterNot { it.id == approvalId } }
        try {
            ShareAccessApi.decide(approvalId, decision)
        } catch (e: CoreException) {
            Log.w(TAG, "share.decideAccessApproval failed: ${e.kind}: ${e.message}")
            if (e.kind != "not_found") Snackbars.show("Antwort nicht übernommen: ${e.message ?: e.kind}")
        } finally {
            answering -= approvalId
            requestRefresh()
        }
    }

    private fun answer(approvalId: String, decision: String, done: () -> Unit) {
        scope.launch {
            answering += approvalId
            pendingFlow.update { list -> list.filterNot { it.id == approvalId } }
            try {
                ShareAccessApi.decide(approvalId, decision)
            } catch (e: CoreException) {
                Log.w(TAG, "share.decideAccessApproval failed: ${e.kind}: ${e.message}")
                if (e.kind != "not_found") Snackbars.show("Antwort nicht übernommen: ${e.message ?: e.kind}")
            } finally {
                answering -= approvalId
                requestRefresh()
                done()
            }
        }
    }

    private suspend fun refresh(context: Context) {
        val all = try {
            ShareAccessApi.approvals()
        } catch (e: CoreException) {
            Log.w(TAG, "share.accessApprovals failed: ${e.kind}: ${e.message}")
            return
        }
        val list = all.filterNot { it.id in answering }
        pendingFlow.value = list
        val open = all.map { it.id }.toSet()
        val openDevices = all.map { deviceGroupKey(it) }.toSet()
        notified.keys.filter { it !in openDevices }.forEach { key -> notified.remove(key)?.let { Notifications.cancel(context, it.id) } }
        reported.retainAll(open)
        val now = SystemClock.elapsedRealtime()
        val since = hiddenSince
        if (startedActivities == 0 && since != null && now - since < HIDDEN_AFTER_MS) {
            // Perhaps only a rotation: decide once the app stayed hidden.
            scope.launch {
                delay(HIDDEN_AFTER_MS - (now - since))
                requestRefresh()
            }
            return
        }
        val visible = startedActivities > 0
        // A connection that arose in the background and waits for its rights (plan §F, F1).
        RightsPendingNotifier.refresh(context, visible)
        for (device in deviceApprovalGroups(list)) {
            for (approval in device.approvals) {
                if (approval.id in reported) continue
                val delivery = deliverApproval(
                    approval,
                    visible,
                    showInApp = { showInApp(now) },
                    post = { postDevice(context, device) },
                    report = { id -> reportUndeliverable(id) },
                )
                if (delivery == ApprovalDelivery.InApp && visible) {
                    notified.remove(device.key)?.let { Notifications.cancel(context, it.id) }
                }
            }
        }
    }

    /** The dialog is up, coming up, or comes up with the next resumed activity. */
    private fun showInApp(now: Long): Boolean {
        if (startedPrompts > 0) return true
        promptLaunchedAt?.let { if (now - it < PROMPT_START_MS) return true }
        val activity = resumed.get() ?: return startedActivities > 0
        return try {
            activity.startActivity(Intent(activity, AccessApprovalActivity::class.java))
            promptLaunchedAt = now
            true
        } catch (e: RuntimeException) {
            Log.w(TAG, "Rückfragedialog nicht geöffnet", e)
            false
        }
    }

    /**
     * Shows [device] as one notification or updates the one it has. A single question is posted on every
     * refresh, as before. A notification of two or more questions is posted again only when their number
     * changes, so one the person swiped away does not come back on every refresh. `true` when it shows
     * the device's questions.
     */
    private fun postDevice(context: Context, device: DeviceApprovals): Boolean {
        val shown = notified[device.key]
        if (device.count > 1 && shown != null && shown.count == device.count) return true
        val id = shown?.id ?: freeApprovalNotificationId(notified.values.map { it.id }) ?: return false
        Notifications.ensureChannels(context)
        val notification = if (device.count == 1) {
            approvalNotification(context, device.approvals.single(), id)
        } else {
            deviceApprovalsNotification(context, device, id)
        }
        if (!Notifications.notify(context, id, notification)) return false
        notified[device.key] = Notice(id, device.count)
        return true
    }

    private suspend fun reportUndeliverable(approvalId: String) {
        reported += approvalId
        try {
            ShareAccessApi.reportUndeliverable(approvalId)
        } catch (e: CoreException) {
            // `not_found`: it ended meanwhile.
            Log.w(TAG, "share.reportApprovalUndeliverable failed: ${e.kind}: ${e.message}")
        }
    }

    /** Which of the app's activities are started; the dialog itself is one of them. */
    private object Visibility : Application.ActivityLifecycleCallbacks {
        override fun onActivityStarted(activity: Activity) {
            startedActivities++
            hiddenSince = null
            if (activity is AccessApprovalActivity) {
                startedPrompts++
                promptLaunchedAt = null
            }
            requestRefresh()
        }

        override fun onActivityStopped(activity: Activity) {
            startedActivities = (startedActivities - 1).coerceAtLeast(0)
            if (activity is AccessApprovalActivity) startedPrompts = (startedPrompts - 1).coerceAtLeast(0)
            if (startedActivities == 0) hiddenSince = SystemClock.elapsedRealtime()
            requestRefresh()
        }

        override fun onActivityResumed(activity: Activity) {
            resumed = WeakReference(activity)
            requestRefresh()
        }

        override fun onActivityPaused(activity: Activity) {
            if (resumed.get() === activity) resumed = WeakReference(null)
        }

        override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) = Unit

        override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) = Unit

        override fun onActivityDestroyed(activity: Activity) = Unit
    }
}
