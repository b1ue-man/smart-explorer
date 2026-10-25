package app.smartexplorer.android.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.net.Uri
import app.smartexplorer.android.api.DECISION_ALWAYS
import app.smartexplorer.android.api.DECISION_DENY
import app.smartexplorer.android.api.DECISION_ONCE

/** Action of the broadcasts of a question's notification (explicit, to this receiver only). */
internal const val ACTION_ACCESS_APPROVAL = "app.smartexplorer.android.action.ACCESS_APPROVAL"

/** Scheme of the data URI that names the question and the answer of one action. */
private const val APPROVAL_SCHEME = "smartexplorer-approval"

private val APPROVAL_ID = Regex("^[0-9a-f]{32}$")
private val DECISIONS = setOf(DECISION_ONCE, DECISION_ALWAYS, DECISION_DENY)

/**
 * `smartexplorer-approval://<id>/<decision>`: each action of a notification carries its own URI, so
 * its PendingIntent is distinct from every other question's and can only ever answer this one.
 */
internal fun approvalUri(approvalId: String, decision: String): Uri =
    Uri.Builder().scheme(APPROVAL_SCHEME).authority(approvalId).appendPath(decision).build()

/** The question and answer an action names; `null` for anything else. */
internal fun approvalAction(uri: Uri?): Pair<String, String>? {
    if (uri == null || uri.scheme != APPROVAL_SCHEME) return null
    val id = uri.authority?.takeIf { APPROVAL_ID.matches(it) } ?: return null
    val decision = uri.pathSegments.singleOrNull()?.takeIf { it in DECISIONS } ?: return null
    return id to decision
}

/**
 * „Diesmal erlauben“, „Immer erlauben“ and „Ablehnen“ of a question's notification, and its swipe
 * away (= „Ablehnen“). Not exported: only this app's immutable PendingIntents reach it. The answer
 * goes to exactly the question the action's URI names (`share.decideAccessApproval`); an action
 * that names none is ignored, so the question stays open and is never allowed by it.
 */
class AccessApprovalReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ACTION_ACCESS_APPROVAL) return
        val (approvalId, decision) = approvalAction(intent.data) ?: return
        if (decision != DECISION_DENY) {
            val current = AccessApprovalNotifier.pending.value.firstOrNull { it.id == approvalId }
            // Contextual approvals require their complete dialog. A stale
            // notification action can never bypass that presentation.
            if (current == null || current.contextual || !current.canAllow) {
                AccessApprovalNotifier.requestRefresh()
                return
            }
        }
        val pending = goAsync()
        AccessApprovalNotifier.decide(approvalId, decision) { pending.finish() }
    }
}
