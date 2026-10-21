package app.smartexplorer.android.ui

import android.content.Context
import android.content.Intent
import app.smartexplorer.android.MainActivity
import java.util.UUID
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow

enum class MainTab { Files, Sync, Share, More }

sealed interface NavRequest {
    data class OpenLocation(val location: String) : NavRequest

    data class SelectTab(val tab: MainTab) : NavRequest

    data object ShowTransfers : NavRequest

    data class OpenJobConflicts(val jobId: String) : NavRequest

    data object ShowShareRequests : NavRequest

    data object ShowUpdate : NavRequest

    data object ShowBackgroundSettings : NavRequest

    /** Opens the screen of a device (SCR-M9); [target] is the target JSON of contract V5. */
    data class OpenScreen(val target: String) : NavRequest

    /** Opens the group view of the screens (SCR-M9) of a room, or of all devices with `null`. */
    data class ShowScreenGrid(val roomId: String?) : NavRequest

    /** Opens the comparison of the vault sync job [jobId] (VLT-M11, U31-8), under „Mehr → Tresore“. */
    data class OpenVaultCompare(val jobId: String) : NavRequest

    /** Opens „Mehr → Einstellungen“ at the card „Startbildschirm“ of the launcher (DSK-M9). */
    data object ShowLauncherSettings : NavRequest
}

/**
 * In-app navigation requests from notifications, services and screens.
 *
 * - [requests]: every request; `AppRoot` collects it to switch the tab.
 * - [pending]: the latest request not yet handled by its target screen. A screen that is only
 *   composed after the tab switch reads it, acts, and calls [consume] – so nothing is lost and
 *   nothing replays after a later tab change.
 */
object AppNav {
    private const val ACTION_NAVIGATE = "app.smartexplorer.android.action.NAVIGATE"
    private const val EXTRA_KIND = "app.smartexplorer.android.extra.NAV_KIND"
    private const val EXTRA_VALUE = "app.smartexplorer.android.extra.NAV_VALUE"
    private const val EXTRA_TOKEN = "app.smartexplorer.android.extra.NAV_TOKEN"
    private const val TOKEN_PREFS = "nav"
    private const val TOKEN_KEY = "token"
    private const val VAULT_JOB_PREFIX = "vault:"

    private val lock = Any()
    private val queued = ArrayDeque<NavRequest>()
    private val requestFlow = MutableSharedFlow<NavRequest>(extraBufferCapacity = 16)
    private val pendingFlow = MutableStateFlow<NavRequest?>(null)

    val requests: SharedFlow<NavRequest> = requestFlow.asSharedFlow()
    val pending: StateFlow<NavRequest?> = pendingFlow.asStateFlow()

    /** Thread-safe; requests sent before `AppRoot` collects are kept until it does. */
    fun send(request: NavRequest) {
        synchronized(lock) {
            pendingFlow.value = request
            if (requestFlow.subscriptionCount.value == 0 || !requestFlow.tryEmit(request)) queued.addLast(request)
        }
    }

    /** Marks [request] as handled by its screen; a newer pending request stays. */
    fun consume(request: NavRequest) {
        pendingFlow.compareAndSet(request, null)
    }

    /** Called by `AppRoot` once its collector runs, to deliver requests sent before. */
    internal fun flushQueued() {
        synchronized(lock) {
            while (queued.isNotEmpty()) {
                if (!requestFlow.tryEmit(queued.first())) return
                queued.removeFirst()
            }
        }
    }

    /** Explicit intent for [MainActivity], e.g. for a notification `PendingIntent`. */
    fun intentFor(context: Context, request: NavRequest): Intent = intentFor(context, request, token(context))

    /**
     * The same with a [token] the caller already holds. The launcher process (`:launcher`) never reads the
     * token store of the main process; it gets the token from the main process and passes it here (U35-14).
     */
    fun intentFor(context: Context, request: NavRequest, token: String): Intent {
        val (kind, value) = encode(request)
        return Intent(context, MainActivity::class.java).apply {
            action = ACTION_NAVIGATE
            putExtra(EXTRA_KIND, kind)
            if (value != null) putExtra(EXTRA_VALUE, value)
            putExtra(EXTRA_TOKEN, token)
            // Distinct identity per request so PendingIntents for different targets never merge.
            identifier = "nav:$kind:${value.orEmpty()}"
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        }
    }

    /**
     * Request carried by an intent from [intentFor], or `null`. MainActivity is exported (launcher,
     * share target), so only intents with this installation's private token are accepted.
     */
    fun fromIntent(context: Context, intent: Intent?): NavRequest? {
        if (intent?.action != ACTION_NAVIGATE) return null
        if (intent.getStringExtra(EXTRA_TOKEN) != token(context)) return null
        return decode(intent.getStringExtra(EXTRA_KIND), intent.getStringExtra(EXTRA_VALUE))
    }

    /** Request of a [kind] and [value] as [encode] writes them, or `null` for an unknown kind or a missing value. */
    internal fun decode(kind: String?, value: String?): NavRequest? = when (kind) {
        "openLocation" -> value?.let { NavRequest.OpenLocation(it) }
        "selectTab" -> MainTab.entries.firstOrNull { it.name == value }?.let { NavRequest.SelectTab(it) }
        "showTransfers" -> NavRequest.ShowTransfers
        // Conflicts of a vault sync job come as `vault:<job>` (U31-13): its comparison.
        "openJobConflicts" -> value?.let { job ->
            if (job.startsWith(VAULT_JOB_PREFIX)) {
                NavRequest.OpenVaultCompare(job.removePrefix(VAULT_JOB_PREFIX))
            } else {
                NavRequest.OpenJobConflicts(job)
            }
        }
        "showShareRequests" -> NavRequest.ShowShareRequests
        "showUpdate" -> NavRequest.ShowUpdate
        "showBackgroundSettings" -> NavRequest.ShowBackgroundSettings
        "openScreen" -> value?.takeIf { it.isNotBlank() }?.let { NavRequest.OpenScreen(it) }
        "showScreenGrid" -> NavRequest.ShowScreenGrid(value?.takeIf { it.isNotBlank() })
        "openVaultCompare" -> value?.takeIf { it.isNotBlank() }?.let { NavRequest.OpenVaultCompare(it) }
        "showLauncherSettings" -> NavRequest.ShowLauncherSettings
        else -> null
    }

    /** Tab that shows the target of [request]. */
    fun tabFor(request: NavRequest): MainTab = when (request) {
        is NavRequest.OpenLocation, NavRequest.ShowTransfers -> MainTab.Files
        is NavRequest.SelectTab -> request.tab
        is NavRequest.OpenJobConflicts -> MainTab.Sync
        NavRequest.ShowShareRequests, is NavRequest.OpenScreen, is NavRequest.ShowScreenGrid -> MainTab.Share
        NavRequest.ShowUpdate, NavRequest.ShowBackgroundSettings, is NavRequest.OpenVaultCompare,
        NavRequest.ShowLauncherSettings -> MainTab.More
    }

    internal fun encode(request: NavRequest): Pair<String, String?> = when (request) {
        is NavRequest.OpenLocation -> "openLocation" to request.location
        is NavRequest.SelectTab -> "selectTab" to request.tab.name
        NavRequest.ShowTransfers -> "showTransfers" to null
        is NavRequest.OpenJobConflicts -> "openJobConflicts" to request.jobId
        NavRequest.ShowShareRequests -> "showShareRequests" to null
        NavRequest.ShowUpdate -> "showUpdate" to null
        NavRequest.ShowBackgroundSettings -> "showBackgroundSettings" to null
        is NavRequest.OpenScreen -> "openScreen" to request.target
        is NavRequest.ShowScreenGrid -> "showScreenGrid" to (request.roomId ?: "")
        is NavRequest.OpenVaultCompare -> "openVaultCompare" to request.jobId
        NavRequest.ShowLauncherSettings -> "showLauncherSettings" to null
    }

    /**
     * The token of this installation for the service that hands it to the launcher process (`LauncherBridgeService`,
     * U40-12). Main process only: the process `:launcher` never calls it (U35-16); it gets the token from the `DEVICES` message.
     */
    internal fun tokenFor(context: Context): String = token(context)

    /** Random per-installation secret in app-private preferences. */
    private fun token(context: Context): String = synchronized(lock) {
        val prefs = context.applicationContext.getSharedPreferences(TOKEN_PREFS, Context.MODE_PRIVATE)
        prefs.getString(TOKEN_KEY, null) ?: UUID.randomUUID().toString().also {
            prefs.edit().putString(TOKEN_KEY, it).apply()
        }
    }
}
