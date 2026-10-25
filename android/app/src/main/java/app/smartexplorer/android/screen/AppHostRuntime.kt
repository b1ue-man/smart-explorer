package app.smartexplorer.android.screen

import android.app.Activity
import android.app.Application
import android.util.Log
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.ui.common.Snackbars
import java.lang.ref.WeakReference
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.*

/** Direct bounded host events; the lossy UI SharedFlow is never their transport. */
internal object AppHostRuntime {
    private const val TAG = "AppHostRuntime"
    private val events = Channel<JsonObject>(64)
    private val identityRefresh = Channel<Unit>(Channel.CONFLATED)
    private val pending = MutableStateFlow<List<PendingAppLaunch>>(emptyList())
    private val busy = MutableStateFlow<Set<String>>(emptySet())
    @Volatile private var application: Application? = null
    @Volatile private var scope: CoroutineScope? = null
    @Volatile private var identity: String? = null
    private var visible = WeakReference<Activity>(null)

    fun attach(app: Application, hostScope: CoroutineScope) {
        if (application != null) return
        application = app; scope = hostScope
        hostScope.launch {
            for (event in events) {
                try {
                    if (identity == null || !AppLaunch.onEvent(event)) reject(event, "android_app_host_identity_not_ready")
                } catch (failure: Exception) {
                    Log.w(TAG, "Local app host event failed", failure)
                    runCatching { reject(event, "android_local_app_host_failed", launchMayHaveDispatched = true) }
                } finally { refreshPending() }
            }
        }
        hostScope.launch {
            var retryMs = 2_000L
            for (signal in identityRefresh) {
                val actual = runCatching {
                    val status = Core.call("share.status") as? JsonObject
                    ((status?.get("identity") as? JsonObject)?.get("deviceId") as? JsonPrimitive)?.contentOrNull
                        ?.takeIf { it.isNotBlank() }
                }.getOrNull()
                if (actual != null) {
                    withContext(Dispatchers.Main.immediate) {
                        AppLaunch.install(app, actual, hostScope)
                        visible.get()?.let(AppLaunch::onActivityResumed)
                    }
                    val registered = runCatching { Core.call("apps.hostRegister") }.isSuccess
                    identity = if (registered) actual else null
                    if (!registered) { delay(retryMs); identityRefresh.trySend(Unit) }
                    retryMs = 2_000L
                    refreshPending()
                } else {
                    // The first native poll may not have produced an identity.
                    // No registration or invented replacement ID occurs here.
                    delay(retryMs)
                    retryMs = (retryMs * 2).coerceAtMost(30_000L)
                    identityRefresh.trySend(Unit)
                }
            }
        }
        identityRefresh.trySend(Unit)
    }

    /** Called on the native event-pump thread, before UI decoding/emission. */
    fun acceptEvent(event: JsonObject): Boolean {
        val type = (event["type"] as? JsonPrimitive)?.contentOrNull
        if (type == "share") identityRefresh.trySend(Unit)
        if (type !in setOf("appsHostCatalog", "appsHostLaunch", "appsHostIcon", "appsHostConsentEnded")) return false
        if (events.trySend(event).isFailure) {
            // Overflow is an explicit negative response to the native challenge,
            // not a dropped launch, fabricated success or automatic replay.
            runBlocking { runCatching {
                if (type == "appsHostConsentEnded") { AppLaunch.onEvent(event); refreshPending() }
                else reject(event, "android_app_host_queue_busy")
            } }
        }
        return true
    }

    private suspend fun reject(event: JsonObject, reason: String, launchMayHaveDispatched: Boolean = false) {
        val challenge = event["challenge"] ?: return
        when ((event["type"] as? JsonPrimitive)?.contentOrNull) {
            "appsHostCatalog" -> Core.call("apps.hostCatalogFailed", buildJsonObject { put("challenge", challenge); put("reason", reason) })
            "appsHostLaunch" -> Core.call("apps.hostLaunchResult", buildJsonObject {
                put("challenge", challenge)
                // Once event processing began, a failed journal/response can follow a real OS start.
                put("state", if (launchMayHaveDispatched) "outcome_unknown" else "denied")
                put("reason", reason)
            })
            "appsHostIcon" -> Core.call("apps.hostIconResult", buildJsonObject { put("challenge", challenge); put("reply", JsonNull) })
        }
    }

    fun onVisible(activity: Activity) {
        visible = WeakReference(activity)
        if (identity != null) AppLaunch.onActivityResumed(activity)
        identityRefresh.trySend(Unit)
        scope?.launch { refreshPending() }
    }
    fun onHidden(activity: Activity) {
        if (visible.get() === activity) visible.clear()
        AppLaunch.onActivityPaused(activity)
    }
    private suspend fun refreshPending() {
        pending.value = runCatching { AppLaunch.pendingConfirmations() }.getOrDefault(emptyList())
    }
    private fun answer(activity: Activity, request: PendingAppLaunch, allow: Boolean) {
        val hostScope = scope ?: return
        synchronized(busy) {
            if (request.requestId in busy.value) return
            busy.value = busy.value + request.requestId
        }
        hostScope.launch {
            try {
                if (allow) AppLaunch.confirmFromUserTap(activity, request.requestId) else AppLaunch.denyFromUserTap(request.requestId)
            } catch (failure: Exception) {
                Snackbars.show(failure.message ?: "Programmstart konnte nicht bestätigt werden.")
            } finally { synchronized(busy) { busy.update { it - request.requestId } }; refreshPending() }
        }
    }

    /** Visible fallback when notifications are unavailable; every action is a local click. */
    @Composable
    fun PendingRequests(activity: Activity, modifier: Modifier = Modifier) {
        val requests by pending.collectAsState()
        val running by busy.collectAsState()
        if (requests.isEmpty()) return
        ElevatedCard(modifier.fillMaxWidth().padding(12.dp)) {
            Column(Modifier.padding(12.dp)) {
                Text("Programmstarts bestätigen (${requests.size})", style = MaterialTheme.typography.titleSmall)
                LazyColumn(Modifier.heightIn(max = (LocalConfiguration.current.screenHeightDp / 3).dp)) {
                    items(requests, key = { it.requestId }) { request ->
                        Row(Modifier.fillMaxWidth()) {
                            Text(request.name, Modifier.weight(1f).padding(vertical = 12.dp))
                            TextButton(enabled = request.requestId !in running, onClick = { answer(activity, request, true) }) { Text("Starten") }
                            TextButton(enabled = request.requestId !in running, onClick = { answer(activity, request, false) }) { Text("Ablehnen") }
                        }
                    }
                }
            }
        }
    }
}
