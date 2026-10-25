package app.smartexplorer.android.screen

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.app.Application
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.LauncherApps
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.Process
import android.os.SystemClock
import android.util.AtomicFile
import app.smartexplorer.android.MainActivity
import app.smartexplorer.android.core.Core
import java.io.File
import java.lang.ref.WeakReference
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.encodeToJsonElement
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlinx.serialization.json.JsonNull

@Serializable
internal data class AppLaunchRequest(
    @SerialName("request_id") val requestId: String,
    @SerialName("target_device_id") val targetDeviceId: String,
    @SerialName("app_id") val appId: String,
    @SerialName("user_id") val userId: String,
    @SerialName("session_id") val sessionId: String,
    @SerialName("catalog_revision") val catalogRevision: String,
)

@Serializable
private data class HostLaunchEvent(
    val type: String,
    val challenge: String,
    val request: AppLaunchRequest,
    @SerialName("expected_session") val expectedSession: AppHostSession,
    val fingerprint: String,
    @SerialName("expires_at_unix_ms") val expiresAtUnixMs: Long? = null,
    @SerialName("dispatch_id") val dispatchId: String,
)

@Serializable
private data class StoredAppLaunch(val request: AppLaunchRequest, val state: String, val reason: String, val name: String)
private data class LocalLaunchAdmission(val unixEnd: Long, val elapsedEnd: Long)

internal data class PendingAppLaunch(val requestId: String, val name: String)

/**
 * Main hooks: install once in the core process, route raw appsHostCatalog/appsHostLaunch events
 * to onEvent, and call showConsentForIntent from MainActivity's normal intent handling.
 * The launcher process, AccessibilityService and foreground service confer no BAL exception.
 */
internal object AppLaunch : Application.ActivityLifecycleCallbacks {
    private const val CHANNEL = "remote-app-launch"
    private const val CONSENT_EXTRA = "se_app_launch_consent"
    private val json = Json { encodeDefaults = true; explicitNulls = false; ignoreUnknownKeys = false }
    private var application: Application? = null
    private var scope: CoroutineScope? = null
    @Volatile private var deviceId: String? = null
    private var resumed = WeakReference<Activity>(null)
    private val approved = mutableSetOf<String>()
    private val diskLock = Any()

    fun install(app: Application, hostDeviceId: String, hostScope: CoroutineScope) {
        check(hostDeviceId.isNotBlank())
        if (application != null) {
            check(application === app)
            if (deviceId != hostDeviceId) {
                deviceId = hostDeviceId
                synchronized(approved) { approved.clear() }
                val manager = app.getSystemService(NotificationManager::class.java)
                manager?.activeNotifications?.filter { it.notification.channelId == CHANNEL }?.forEach { manager.cancel(it.id) }
                hostScope.launch { Core.call("apps.hostRegister") }
            }
            return
        }
        application = app; deviceId = hostDeviceId; scope = hostScope
        app.registerActivityLifecycleCallbacks(this)
        val notifications = app.getSystemService(NotificationManager::class.java)
        notifications?.createNotificationChannel(NotificationChannel(CHANNEL, "Programmstarts bestätigen", NotificationManager.IMPORTANCE_DEFAULT))
        hostScope.launch { Core.call("apps.hostRegister") }
    }

    /** Input is from the local JNI event pump, not a network message handler. */
    suspend fun onEvent(event: JsonObject): Boolean {
        val app = application ?: return false
        return when (event["type"]?.jsonPrimitive?.contentOrNull) {
            "appsHostCatalog" -> {
                val challenge = event["challenge"]?.jsonPrimitive?.contentOrNull ?: return false
                // Catalogs carry references only. PNG payloads use the separate
                // bounded appsHostIcon challenge, including on the local JNI hop.
                val catalog = withContext(Dispatchers.IO) { runCatching { AppCatalog.read(app).wire.copy(icons = emptyList()) } }
                catalog.fold(onSuccess = { result ->
                    Core.call("apps.hostCatalog", buildJsonObject {
                        put("challenge", challenge); put("catalog", json.encodeToJsonElement(result))
                    })
                }, onFailure = {
                    Core.call("apps.hostCatalogFailed", buildJsonObject {
                        put("challenge", challenge); put("reason", "android_launcher_catalog_unavailable")
                    })
                })
                true
            }
            "appsHostLaunch" -> {
                val request = json.decodeFromJsonElement<HostLaunchEvent>(event)
                val receipt = dispatch(app, request)
                Core.call("apps.hostLaunchResult", buildJsonObject {
                    put("challenge", request.challenge); put("state", receipt.state); put("reason", receipt.reason)
                })
                true
            }
            "appsHostIcon" -> {
                val challenge = event["challenge"]?.jsonPrimitive?.contentOrNull ?: return false
                val expected = json.decodeFromJsonElement<ScreenIconRef>(event["icon"] ?: return false)
                val id = event["app_id"]?.jsonPrimitive?.contentOrNull ?: return false
                val reply = withContext(Dispatchers.IO) {
                    val catalog = AppCatalog.read(app)
                    catalog.wire.icons.firstOrNull { it.icon == expected && it.icon.appId == id && catalog.targets.containsKey(id) }
                }
                Core.call("apps.hostIconResult", buildJsonObject {
                    put("challenge", challenge); put("reply", reply?.let { json.encodeToJsonElement(it) } ?: JsonNull)
                })
                true
            }
            "appsHostConsentEnded" -> {
                val request = json.decodeFromJsonElement<AppLaunchRequest>(event["request"] ?: return false)
                withContext(Dispatchers.IO) { synchronized(diskLock) {
                    val record = read(app, request.requestId)
                    if (record != null && record.request == request && record.state == "consent_required") {
                        write(app, record.copy(state = "denied", reason = "native_app_consent_no_longer_authorized"))
                    }
                } }
                synchronized(approved) { approved.remove(request.requestId) }
                app.getSystemService(NotificationManager::class.java)?.cancel(notificationId(request.requestId))
                true
            }
            else -> false
        }
    }

    private fun file(context: Context, id: String): AtomicFile {
        val root = File(context.filesDir, "apps/launch-receipts")
        check(root.isDirectory || root.mkdirs()) { "Android app launch ledger unavailable" }
        return AtomicFile(File(root, AppCatalog.hash(id) + ".json"))
    }
    private fun read(context: Context, id: String): StoredAppLaunch? {
        val file = file(context, id)
        return if (!file.baseFile.exists()) null else {
            check(file.baseFile.length() <= 32 * 1024) { "Android launch receipt exceeds its byte budget" }
            json.decodeFromString<StoredAppLaunch>(file.readFully().toString(Charsets.UTF_8))
        }
    }
    private fun write(context: Context, record: StoredAppLaunch) {
        val file = file(context, record.request.requestId)
        val bytes = json.encodeToString(record).toByteArray(Charsets.UTF_8)
        check(bytes.size <= 32 * 1024) { "Android launch receipt exceeds its byte budget" }
        val output = file.startWrite()
        try { output.write(bytes); file.finishWrite(output) } catch (failure: Exception) {
            file.failWrite(output); throw failure
        }
    }

    private suspend fun dispatch(context: Context, event: HostLaunchEvent): StoredAppLaunch {
        val request = event.request
        val catalog = withContext(Dispatchers.IO) { runCatching { AppCatalog.read(context) } }.getOrElse {
            return StoredAppLaunch(request, "unsupported", "android_launcher_catalog_unavailable", request.appId)
        }
        val target = catalog.targets[request.appId]
        val reject = when {
            request.targetDeviceId != deviceId -> "target_device_mismatch"
            request.userId != catalog.wire.session.userId || request.sessionId != catalog.wire.session.sessionId -> "android_user_or_login_session_changed"
            event.expectedSession != catalog.wire.session -> "android_session_changed_before_dispatch"
            target == null || target.entry.fingerprint != event.fingerprint -> "android_installed_launcher_catalog_changed"
            !target.entry.app.canLaunch -> "android_launcher_activity_not_available"
            else -> null
        }
        val name = target?.entry?.app?.name ?: request.appId
        val prepared = withContext(Dispatchers.Main.immediate) {
            synchronized(diskLock) {
                val old = read(context, request.requestId)
                if (old != null && old.request != request) return@synchronized StoredAppLaunch(request, "denied", "request_id_reused_with_different_request", name) to false
                if (old != null && old.state != "consent_required") return@synchronized old to false
                if (reject != null) return@synchronized StoredAppLaunch(request, "denied", reject, name).also { write(context, it) } to false
                // Visible Activity is the only automatically proven exception.
                // Role HOME, Accessibility and a foreground service are insufficient.
                val activity = visibleActivity()
                val localConfirmed = synchronized(approved) { approved.remove(request.requestId) }
                if (activity == null || AppCatalog.session(context) != event.expectedSession ||
                    (old?.state == "consent_required" && !localConfirmed)) {
                    val consent = StoredAppLaunch(request, "consent_required", "local_visible_unlocked_host_confirmation_required", name)
                    write(context, consent)
                    val posted = runCatching { notifyConsent(context, consent) }.getOrDefault(false)
                    val actual = if (posted) consent else consent.copy(reason = "local_confirmation_required_open_smart_explorer_notifications_unavailable")
                    if (actual != consent) write(context, actual)
                    return@synchronized actual to false
                }
                val pending = StoredAppLaunch(request, "outcome_unknown", "android_os_start_result_not_yet_confirmed", name)
                if (event.expiresAtUnixMs?.let { System.currentTimeMillis() >= it } == true) {
                    return@synchronized pending.copy(state = "denied", reason = "app_launch_grant_expired").also { write(context, it) } to false
                }
                write(context, pending) // Durable before LauncherApps IPC, including process death.
                pending to true
            }
        }
        if (!prepared.second) return prepared.first
        val pending = prepared.first
        // The native guard is held through this one-use invocation window. The
        // monotonic deadline starts BEFORE JNI, conservatively charging its delay.
        val admissionStarted = SystemClock.elapsedRealtime()
        val admission = runCatching {
            val reply = Core.call("apps.hostAdmission", buildJsonObject {
                put("dispatch_id", event.dispatchId); put("request_id", request.requestId)
            }) as? JsonObject ?: error("Invalid local admission reply")
            check(reply["allowed"]?.jsonPrimitive?.contentOrNull == "true")
            val unixEnd = reply["expires_at_unix_ms"]?.jsonPrimitive?.contentOrNull?.toLongOrNull() ?: error("Missing admission end")
            val validFor = reply["valid_for_ms"]?.jsonPrimitive?.contentOrNull?.toLongOrNull() ?: error("Missing admission window")
            check(validFor in 1L..2000L)
            LocalLaunchAdmission(unixEnd, admissionStarted + validFor)
        }.getOrNull()
        return withContext(Dispatchers.Main.immediate) {
            synchronized(diskLock) {
                val current = read(context, request.requestId)
                if (current == null || current.request != request || current.state != "outcome_unknown") return@synchronized current ?: pending
                if (admission == null || System.currentTimeMillis() >= admission.unixEnd || SystemClock.elapsedRealtime() >= admission.elapsedEnd ||
                    event.expiresAtUnixMs?.let { System.currentTimeMillis() >= it } == true ||
                    AppCatalog.session(context) != event.expectedSession || visibleActivity() == null) {
                    return@synchronized pending.copy(state = "denied", reason = "android_launch_admission_expired_or_session_changed").also { write(context, it) }
                }
                val result = try {
                    val launcher = requireNotNull(context.getSystemService(LauncherApps::class.java))
                    launcher.startMainActivity(requireNotNull(target).component, Process.myUserHandle(), null, null)
                    pending.copy(state = "started_window_pending", reason = "android_launcher_start_accepted_window_not_yet_available")
                } catch (failure: SecurityException) {
                    pending.copy(state = "denied", reason = "android_platform_denied_activity_start")
                } catch (failure: android.content.ActivityNotFoundException) {
                    pending.copy(state = "unsupported", reason = "android_launcher_activity_removed")
                } catch (failure: Exception) {
                    pending.copy(state = "outcome_unknown", reason = "android_os_start_result_unknown")
                }
                write(context, result)
                context.getSystemService(NotificationManager::class.java)?.cancel(notificationId(request.requestId))
                result
            }
        }
    }

    private fun visibleActivity(): Activity? {
        val activity = resumed.get() ?: return null
        val app = application ?: return null
        return activity.takeIf { !it.isFinishing && !it.isDestroyed && it.hasWindowFocus() &&
            it.window.decorView.isShown && AppCatalog.session(app).let { s -> s.interactive && s.unlocked } }
    }
    private fun notificationId(id: String): Int = AppCatalog.hash(id).take(7).toInt(16)
    private fun notifyConsent(context: Context, record: StoredAppLaunch): Boolean {
        if (Build.VERSION.SDK_INT >= 33 && context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return false
        val manager = context.getSystemService(NotificationManager::class.java) ?: return false
        if (!manager.areNotificationsEnabled() || manager.getNotificationChannel(CHANNEL)?.importance == NotificationManager.IMPORTANCE_NONE) return false
        val intent = Intent(context, MainActivity::class.java).putExtra(CONSENT_EXTRA, record.request.requestId)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        val content = PendingIntent.getActivity(context, notificationId(record.request.requestId), intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val notification = Notification.Builder(context, CHANNEL).setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle("Programmstart bestätigen").setContentText(record.name)
            .setContentIntent(content).setAutoCancel(true).build()
        manager.notify(notificationId(record.request.requestId), notification)
        return true
    }

    /** Call only from a visible local UI action; Main performs fresh E4 admission. */
    suspend fun confirmFromUserTap(activity: Activity, id: String) {
        check(withContext(Dispatchers.Main.immediate) {
            resumed.get() === activity && !activity.isFinishing && !activity.isDestroyed &&
                activity.window.decorView.isShown && AppCatalog.session(activity).let { it.interactive && it.unlocked }
        }) { "Host activity must be visibly resumed and unlocked" }
        val app = requireNotNull(application)
        val record = withContext(Dispatchers.IO) { synchronized(diskLock) { read(app, id) } }
            ?: error("No pending local app launch confirmation")
        check(record.state == "consent_required") { "No pending local app launch confirmation" }
        val current = AppCatalog.session(app)
        check(record.request.targetDeviceId == deviceId && record.request.userId == current.userId && record.request.sessionId == current.sessionId) {
            "Host identity or session changed; reload the request"
        }
        synchronized(approved) { approved.add(id) }
        try { Core.call("apps.hostConsent", buildJsonObject { put("request_id", id) }) }
        catch (failure: Exception) { synchronized(approved) { approved.remove(id) }; throw failure }
    }

    /** Notification opens the app's own Activity; an actual local dialog grants continuation. */
    fun showConsentForIntent(activity: Activity, intent: Intent?): Boolean {
        val id = intent?.getStringExtra(CONSENT_EXTRA) ?: return false
        val app = application ?: return false
        val hostScope = scope ?: return false
        intent.removeExtra(CONSENT_EXTRA)
        hostScope.launch {
            val record = withContext(Dispatchers.IO) { synchronized(diskLock) { read(app, id) } } ?: return@launch
            if (record.state != "consent_required") return@launch
            if (record.request.targetDeviceId != deviceId) return@launch
            withContext(Dispatchers.Main.immediate) {
                AlertDialog.Builder(activity).setTitle("Programmstart bestätigen")
                    .setMessage("„${record.name}“ auf diesem Gerät starten?")
                    .setNegativeButton("Abbrechen") { _, _ -> hostScope.launch { denyFromUserTap(id) } }
                    .setPositiveButton("Starten") { dialog, _ ->
                        dialog.dismiss()
                        activity.window.decorView.post { hostScope.launch { confirmFromUserTap(activity, id) } }
                    }.show()
            }
        }
        return true
    }
    suspend fun denyFromUserTap(id: String) {
        val app = requireNotNull(application)
        withContext(Dispatchers.IO) { synchronized(diskLock) {
            val record = read(app, id) ?: return@synchronized
            if (record.state == "consent_required") write(app, record.copy(state = "denied", reason = "host_user_refused_application_start"))
        } }
        Core.call("apps.hostDeny", buildJsonObject { put("request_id", id) })
        app.getSystemService(NotificationManager::class.java)?.cancel(notificationId(id))
    }
    /** A visible in-app list remains available when notifications are disabled. */
    suspend fun pendingConfirmations(): List<PendingAppLaunch> {
        val app = application ?: return emptyList()
        return withContext(Dispatchers.IO) { synchronized(diskLock) {
            val session = AppCatalog.session(app)
            File(app.filesDir, "apps/launch-receipts").listFiles().orEmpty()
                .filter { it.isFile && it.extension == "json" && it.length() <= 32 * 1024 }
                .mapNotNull { runCatching { json.decodeFromString<StoredAppLaunch>(it.readText()) }.getOrNull() }
                .filter { it.state == "consent_required" && it.request.sessionId == session.sessionId &&
                    it.request.userId == session.userId && it.request.targetDeviceId == deviceId &&
                    synchronized(approved) { it.request.requestId !in approved } }
                .map { PendingAppLaunch(it.request.requestId, it.name) }
        } }
    }

    override fun onActivityResumed(activity: Activity) { resumed = WeakReference(activity) }
    override fun onActivityPaused(activity: Activity) { if (resumed.get() === activity) resumed.clear() }
    override fun onActivityCreated(activity: Activity, state: Bundle?) = Unit
    override fun onActivityStarted(activity: Activity) = Unit
    override fun onActivityStopped(activity: Activity) = Unit
    override fun onActivitySaveInstanceState(activity: Activity, state: Bundle) = Unit
    override fun onActivityDestroyed(activity: Activity) { if (resumed.get() === activity) resumed.clear() }
}
