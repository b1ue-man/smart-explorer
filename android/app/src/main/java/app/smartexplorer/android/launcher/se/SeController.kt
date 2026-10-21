package app.smartexplorer.android.launcher.se

import android.app.Application
import android.content.Context
import android.content.Intent
import android.util.Log
import app.smartexplorer.android.launcher.LauncherState
import app.smartexplorer.android.launcher.bridge.DevicesData
import app.smartexplorer.android.launcher.bridge.GlobalResult
import app.smartexplorer.android.launcher.model.LayoutRepository
import app.smartexplorer.android.ui.AppNav
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/**
 * The launcher's side of the se elements (DSK-M13): the bridge client, the cache, the current [SeUiState], the first
 * fill and the opening of targets. One instance in the process `:launcher`. It binds the main process's service only
 * by rule C3.3 (U40-7) and never calls `AppNav.tokenFor`; the token comes from `DEVICES`.
 *
 * The activity forwards its life cycle ([onStart], [onResume], [onStop]); the home screen tells whether the visible
 * page or the dock carries a se element ([setVisibleHasSe]).
 */
class SeController private constructor(private val app: Application) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val cache = SeCache(LauncherState.dir(app), scope)
    private val client = SeBridgeClient(app, ::onData)
    private val _ui = MutableStateFlow(SeUiState())

    /** The data to draw: live from the service, else the cache with `stale` true. */
    val ui: StateFlow<SeUiState> = _ui.asStateFlow()

    private val _messages = MutableSharedFlow<String>(extraBufferCapacity = 4)

    /** Short messages for the bottom edge of the home screen (the toast of the view model shows them). */
    val messages: SharedFlow<String> = _messages.asSharedFlow()

    /** The application context, for starting a target from the search (the visible activity is not at hand there). */
    val appContext: Context get() = app

    private var started = false
    private var visibleHasSe = false

    init {
        scope.launch(Dispatchers.IO) {
            val cached = cache.read()
            if (cached != null) _ui.update { if (it.data == null) it.copy(data = cached) else it }
        }
        scope.launch {
            client.state.collect { state ->
                // Once the connection is gone, what is held in memory is no longer current.
                val keep = state is BridgeState.Connected || state is BridgeState.Binding
                _ui.update { it.copy(bridge = state, data = if (keep) it.data else it.data?.copy(stale = true)) }
            }
        }
    }

    // ---- life cycle and the binding rule C3.3 -------------------------------------------------------

    fun onStart() {
        started = true
        // Whatever stayed in memory from an earlier run is not current any more.
        _ui.update { it.copy(data = it.data?.copy(stale = true)) }
    }

    /** Rule (a), checked on every `onResume` and when the visible page changes. */
    fun onResume() = evaluate()

    /** Called whenever the visible page changes, with whether the page or the dock carries a se element (rule C3.3 a). */
    fun setVisibleHasSe(has: Boolean) {
        visibleHasSe = has
        evaluate()
    }

    fun onStop() {
        started = false
        client.unbind()
        _ui.update { it.copy(data = it.data?.copy(stale = true)) }
    }

    private fun evaluate() {
        if (started && visibleHasSe && SeBridgeClient.isMainProcessRunning(app)) client.bind()
    }

    /** Rule (b): the person asks for the connection („Verbinden“, „Erneut“). Starts the main process if it is not running. */
    fun connect() {
        client.bind()
    }

    // ---- data ------------------------------------------------------------------------------------

    private fun onData(data: DevicesData) {
        _ui.update { it.copy(data = data) }
        cache.write(data)
        scope.launch {
            // The first current DEVICES seeds the home screen once (U40-10); the stored names follow later ones.
            LayoutRepository.get(app).update { layout ->
                val base = if (SeSeeder.isSeeded(layout)) layout else SeSeeder.seed(layout, data)
                SeSeeder.syncNames(base, data)
            }
        }
    }

    /** Adds the chosen elements of the sheet „se-Elemente“ to the first free cells of the visible page. */
    fun addEntries(entries: List<SeEntry>) {
        if (entries.isEmpty()) return
        scope.launch {
            val repository = LayoutRepository.get(app)
            repository.update { SeSeeder.add(it, entries, repository.visiblePage) }
        }
    }

    // ---- opening -----------------------------------------------------------------------------------

    /**
     * Starts the main process's screen for [target] from the visible [context] (U40-11): binds (the person's act), waits at
     * most 3000 ms for the token, then starts the explicit intent. Without a token the app opens without a target.
     */
    fun open(context: Context, target: NavTarget) {
        client.bind()
        scope.launch {
            val token = client.awaitToken(SeBridgeClient.FIRST_DATA_TIMEOUT_MS)
            val request = AppNav.decode(target.kind, target.value)
            val intent = if (token != null && request != null) {
                AppNav.intentFor(context, request, token)
            } else {
                app.packageManager.getLaunchIntentForPackage(app.packageName)?.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            }
            if (intent == null) {
                _messages.tryEmit("Smart Explorer ist nicht verfügbar.")
                return@launch
            }
            start(context, intent)
        }
    }

    fun start(context: Context, intent: Intent) {
        try {
            context.startActivity(intent)
        } catch (e: Exception) {
            Log.w(TAG, "start failed", e)
            _messages.tryEmit("Das Ziel ist nicht verfügbar.")
        }
    }

    fun message(text: String) {
        _messages.tryEmit(text)
    }

    /** Sends a global action through the service (the gestures, U40-18); `null` means no answer in time. */
    suspend fun globalAction(action: String): GlobalResult? = client.sendGlobalAction(action)

    companion object {
        private const val TAG = "SeController"

        @Volatile
        private var instance: SeController? = null

        fun get(context: Context): SeController = instance ?: synchronized(this) {
            instance ?: SeController(context.applicationContext as Application).also { instance = it }
        }
    }
}
