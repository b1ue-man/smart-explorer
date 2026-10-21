package app.smartexplorer.android.launcher.model

import android.content.Context
import android.util.Log
import app.smartexplorer.android.launcher.LauncherState
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/**
 * The one holder of the arrangement in the `:launcher` process (U35-36). `LauncherViewModel`, the pin
 * request activity (U38) and the receiver of the se items (U40) use only this object, because the
 * pin request can run while `LauncherActivity` does not live.
 */
class LayoutRepository private constructor(context: Context) {
    private val store = LayoutStore(LauncherState.dir(context))
    private val mutex = Mutex()
    private val state: MutableStateFlow<HomeLayout>

    /** Page the person looks at; [addItem] starts there. Set by the home screen. */
    @Volatile
    var visiblePage: Int = 0

    init {
        val initial = when (val read = store.load()) {
            is LayoutReadResult.Loaded -> read.layout
            LayoutReadResult.Missing -> FirstRun.build(context).also { store.save(it) }
            is LayoutReadResult.Corrupt -> {
                Log.w(TAG, "layout.json set aside: ${read.reason}")
                store.setAsideCorrupt()
                FirstRun.build(context).also { store.save(it) }
            }
        }
        state = MutableStateFlow(initial)
    }

    /** The stored arrangement; sizes are applied only when it is displayed (U35-19). */
    val layout: StateFlow<HomeLayout> = state.asStateFlow()

    /** Applies [transform] to the stored layout, one change at a time, and queues the result for writing. */
    suspend fun update(transform: (HomeLayout) -> HomeLayout): HomeLayout = mutex.withLock {
        val next = transform(state.value)
        if (next != state.value) {
            state.value = next
            store.save(next)
        }
        next
    }

    /**
     * Puts [item] on the visible page when it has room, else on the next page with room, else on a new
     * page. Always `true`, since a page can always be created.
     */
    suspend fun addItem(item: HomeItem): Boolean {
        update { LayoutOps.addAnywhere(it, item, visiblePage) }
        return true
    }

    /** Finishes writing synchronously (`onStop`). */
    fun flush() = store.flush()

    companion object {
        private const val TAG = "LayoutRepository"

        @Volatile
        private var instance: LayoutRepository? = null

        /** One instance per process; it loads the file on the first call. */
        fun get(context: Context): LayoutRepository = instance ?: synchronized(this) {
            instance ?: LayoutRepository(context.applicationContext).also { instance = it }
        }
    }
}
