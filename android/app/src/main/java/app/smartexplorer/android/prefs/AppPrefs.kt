package app.smartexplorer.android.prefs

import android.content.Context
import android.content.SharedPreferences
import androidx.core.content.edit
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * App settings in private SharedPreferences, each exposed as a [StateFlow]. [init] runs in
 * `SmartExplorerApp.onCreate`; before that the flows hold the defaults and setters fail.
 */
object AppPrefs {
    private const val FILE = "app_prefs"
    const val MIN_BG_INTERVAL_MIN = 15

    private val THEMES = setOf("system", "light", "dark")
    private val BG_MODES = setOf("off", "periodic", "persistent")

    @Volatile
    private var prefs: SharedPreferences? = null

    private val themeFlow = MutableStateFlow("system")
    private val showHiddenFlow = MutableStateFlow(false)
    private val dirsFirstFlow = MutableStateFlow(true)
    private val compactFlow = MutableStateFlow(false)
    private val thumbnailsFlow = MutableStateFlow(true)
    private val bgModeFlow = MutableStateFlow("periodic")
    private val bgIntervalMinFlow = MutableStateFlow(60)
    private val bgWifiOnlyFlow = MutableStateFlow(true)
    private val bgChargingOnlyFlow = MutableStateFlow(false)
    private val bgBatteryNotLowFlow = MutableStateFlow(true)
    private val autoUpdateCheckFlow = MutableStateFlow(true)
    private val lastUpdateCheckMsFlow = MutableStateFlow(0L)
    private val onboardingDoneFlow = MutableStateFlow(false)

    /** `system|light|dark` */
    val theme: StateFlow<String> = themeFlow.asStateFlow()
    val showHidden: StateFlow<Boolean> = showHiddenFlow.asStateFlow()
    val dirsFirst: StateFlow<Boolean> = dirsFirstFlow.asStateFlow()
    val compact: StateFlow<Boolean> = compactFlow.asStateFlow()
    val thumbnails: StateFlow<Boolean> = thumbnailsFlow.asStateFlow()

    /** `off|periodic|persistent`; default "Periodisch, 60 min, nur WLAN" (spec). */
    val bgMode: StateFlow<String> = bgModeFlow.asStateFlow()
    val bgIntervalMin: StateFlow<Int> = bgIntervalMinFlow.asStateFlow()
    val bgWifiOnly: StateFlow<Boolean> = bgWifiOnlyFlow.asStateFlow()
    val bgChargingOnly: StateFlow<Boolean> = bgChargingOnlyFlow.asStateFlow()
    val bgBatteryNotLow: StateFlow<Boolean> = bgBatteryNotLowFlow.asStateFlow()
    val autoUpdateCheck: StateFlow<Boolean> = autoUpdateCheckFlow.asStateFlow()
    val lastUpdateCheckMs: StateFlow<Long> = lastUpdateCheckMsFlow.asStateFlow()
    val onboardingDone: StateFlow<Boolean> = onboardingDoneFlow.asStateFlow()

    @Synchronized
    fun init(context: Context) {
        if (prefs != null) return
        val p = context.applicationContext.getSharedPreferences(FILE, Context.MODE_PRIVATE)
        themeFlow.value = p.getString(KEY_THEME, themeFlow.value)?.takeIf { it in THEMES } ?: themeFlow.value
        showHiddenFlow.value = p.getBoolean(KEY_SHOW_HIDDEN, showHiddenFlow.value)
        dirsFirstFlow.value = p.getBoolean(KEY_DIRS_FIRST, dirsFirstFlow.value)
        compactFlow.value = p.getBoolean(KEY_COMPACT, compactFlow.value)
        thumbnailsFlow.value = p.getBoolean(KEY_THUMBNAILS, thumbnailsFlow.value)
        bgModeFlow.value = p.getString(KEY_BG_MODE, bgModeFlow.value)?.takeIf { it in BG_MODES } ?: bgModeFlow.value
        bgIntervalMinFlow.value = p.getInt(KEY_BG_INTERVAL_MIN, bgIntervalMinFlow.value).coerceAtLeast(MIN_BG_INTERVAL_MIN)
        bgWifiOnlyFlow.value = p.getBoolean(KEY_BG_WIFI_ONLY, bgWifiOnlyFlow.value)
        bgChargingOnlyFlow.value = p.getBoolean(KEY_BG_CHARGING_ONLY, bgChargingOnlyFlow.value)
        bgBatteryNotLowFlow.value = p.getBoolean(KEY_BG_BATTERY_NOT_LOW, bgBatteryNotLowFlow.value)
        autoUpdateCheckFlow.value = p.getBoolean(KEY_AUTO_UPDATE_CHECK, autoUpdateCheckFlow.value)
        lastUpdateCheckMsFlow.value = p.getLong(KEY_LAST_UPDATE_CHECK_MS, lastUpdateCheckMsFlow.value)
        onboardingDoneFlow.value = p.getBoolean(KEY_ONBOARDING_DONE, onboardingDoneFlow.value)
        prefs = p
    }

    fun setTheme(value: String) {
        require(value in THEMES) { "unknown theme '$value'" }
        themeFlow.value = value
        store { putString(KEY_THEME, value) }
    }

    fun setShowHidden(value: Boolean) = setBoolean(showHiddenFlow, KEY_SHOW_HIDDEN, value)

    fun setDirsFirst(value: Boolean) = setBoolean(dirsFirstFlow, KEY_DIRS_FIRST, value)

    fun setCompact(value: Boolean) = setBoolean(compactFlow, KEY_COMPACT, value)

    fun setThumbnails(value: Boolean) = setBoolean(thumbnailsFlow, KEY_THUMBNAILS, value)

    fun setBgMode(value: String) {
        require(value in BG_MODES) { "unknown background mode '$value'" }
        bgModeFlow.value = value
        store { putString(KEY_BG_MODE, value) }
    }

    /** Periodic work cannot run more often than every 15 minutes; smaller values are raised. */
    fun setBgIntervalMin(value: Int) {
        val minutes = value.coerceAtLeast(MIN_BG_INTERVAL_MIN)
        bgIntervalMinFlow.value = minutes
        store { putInt(KEY_BG_INTERVAL_MIN, minutes) }
    }

    fun setBgWifiOnly(value: Boolean) = setBoolean(bgWifiOnlyFlow, KEY_BG_WIFI_ONLY, value)

    fun setBgChargingOnly(value: Boolean) = setBoolean(bgChargingOnlyFlow, KEY_BG_CHARGING_ONLY, value)

    fun setBgBatteryNotLow(value: Boolean) = setBoolean(bgBatteryNotLowFlow, KEY_BG_BATTERY_NOT_LOW, value)

    fun setAutoUpdateCheck(value: Boolean) = setBoolean(autoUpdateCheckFlow, KEY_AUTO_UPDATE_CHECK, value)

    fun setLastUpdateCheckMs(value: Long) {
        lastUpdateCheckMsFlow.value = value
        store { putLong(KEY_LAST_UPDATE_CHECK_MS, value) }
    }

    fun setOnboardingDone(value: Boolean) = setBoolean(onboardingDoneFlow, KEY_ONBOARDING_DONE, value)

    private fun setBoolean(flow: MutableStateFlow<Boolean>, key: String, value: Boolean) {
        flow.value = value
        store { putBoolean(key, value) }
    }

    private inline fun store(crossinline write: SharedPreferences.Editor.() -> Unit) {
        val p = checkNotNull(prefs) { "AppPrefs.init was not called" }
        p.edit { write() }
    }

    private const val KEY_THEME = "theme"
    private const val KEY_SHOW_HIDDEN = "show_hidden"
    private const val KEY_DIRS_FIRST = "dirs_first"
    private const val KEY_COMPACT = "compact"
    private const val KEY_THUMBNAILS = "thumbnails"
    private const val KEY_BG_MODE = "bg_mode"
    private const val KEY_BG_INTERVAL_MIN = "bg_interval_min"
    private const val KEY_BG_WIFI_ONLY = "bg_wifi_only"
    private const val KEY_BG_CHARGING_ONLY = "bg_charging_only"
    private const val KEY_BG_BATTERY_NOT_LOW = "bg_battery_not_low"
    private const val KEY_AUTO_UPDATE_CHECK = "auto_update_check"
    private const val KEY_LAST_UPDATE_CHECK_MS = "last_update_check_ms"
    private const val KEY_ONBOARDING_DONE = "onboarding_done"
}
