package app.smartexplorer.android.launcher.widgets

import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProviderInfo
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.LauncherApps
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.UserHandle
import android.os.UserManager
import android.util.SizeF
import app.smartexplorer.android.launcher.model.Geometry
import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.model.Span

/** Request code of the system dialog that binds a widget (U38-7). */
const val REQ_WIDGET_BIND = 0x5E59

/** Request code of a provider's configuration activity (U38-7). */
const val REQ_WIDGET_CONFIGURE = 0x5E58

/**
 * A widget on its way to the home screen (U38-7): the allocated [id], the [provider] as `<package>/<class>`, the
 * profile's serial number and the [span] it will get. [itemId] names the item of the layout this belongs to when
 * an existing widget is bound again or set up; it is `null` for a widget that is being added. The view model keeps
 * it in its `SavedStateHandle` as [encode]d text, so it survives the death of the process during a system dialog.
 */
data class PendingWidget(
    val id: Int,
    val provider: String,
    val userSerial: Long,
    val span: Span,
    val stage: Stage,
    val itemId: String?,
) {
    enum class Stage { Bind, Configure }

    fun encode(): String = listOf(id, provider, userSerial, span.w, span.h, stage.name, itemId.orEmpty()).joinToString(SEPARATOR)

    companion object {
        private const val SEPARATOR = "|"

        fun decode(text: String?): PendingWidget? {
            val parts = text?.split(SEPARATOR) ?: return null
            if (parts.size != 7) return null
            return PendingWidget(
                id = parts[0].toIntOrNull() ?: return null,
                provider = parts[1].takeIf { it.contains('/') } ?: return null,
                userSerial = parts[2].toLongOrNull() ?: return null,
                span = Span(parts[3].toIntOrNull()?.coerceAtLeast(1) ?: return null, parts[4].toIntOrNull()?.coerceAtLeast(1) ?: return null),
                stage = PendingWidget.Stage.entries.firstOrNull { it.name == parts[5] } ?: return null,
                itemId = parts[6].ifEmpty { null },
            )
        }
    }
}

/** What the person or the system has to do next, or the end of the flow. */
sealed interface WidgetStep {
    /** Start the system dialog with [intent] for [REQ_WIDGET_BIND]. */
    data class AskBind(val pending: PendingWidget, val intent: Intent) : WidgetStep

    /** Start the provider's configuration activity for [REQ_WIDGET_CONFIGURE]. */
    data class AskConfigure(val pending: PendingWidget) : WidgetStep

    /** The widget is bound (and set up where needed); place it or write it to its item. */
    data class Finished(val pending: PendingWidget) : WidgetStep

    /** The flow ended without a widget; [message] is shown when it is not `null`. The id is already freed. */
    data class Failed(val message: String?) : WidgetStep
}

/** What [WidgetFlow.restore] decided for one widget item of the layout (U38-6). */
sealed interface Restored {
    data object Keep : Restored

    /** The widget has a new id; [configured] is `false` when it still has to be set up. */
    data class Rebound(val widgetId: Int, val configured: Boolean) : Restored

    /** The approval „Immer erlauben“ is missing: the cell shows „Widget erlauben“. */
    data object NeedsAllow : Restored

    /** The provider is gone. */
    data object Gone : Restored
}

/** The pure part of the restoration decision (U38-6 steps 1, 2 and 4). */
enum class RestoreCase { Keep, Rebind, Remove }

object WidgetRestore {
    /**
     * [boundProvider] is the provider the saved id is bound to now (`null` when the id is unknown), [installed] says
     * the provider still exists, [profileReachable] that its profile could be asked (a locked or paused profile cannot,
     * so nothing is removed for it).
     */
    fun classify(boundProvider: String?, wantedProvider: String, installed: Boolean, profileReachable: Boolean): RestoreCase = when {
        boundProvider == wantedProvider -> RestoreCase.Keep
        !installed && profileReachable -> RestoreCase.Remove
        !installed -> RestoreCase.Keep
        else -> RestoreCase.Rebind
    }
}

/**
 * Binding, setting up and restoring widgets (U38-6 to U38-8). The class makes no system dialog itself: every method
 * that needs one returns it as a [WidgetStep], and the activity — `LauncherActivity` or `PinRequestActivity` — runs it
 * and reports the result back. That keeps the flow the same for both entries.
 */
class WidgetFlow(context: Context, val host: LauncherWidgetHost) {
    private val app = context.applicationContext
    val manager: AppWidgetManager = AppWidgetManager.getInstance(app)
    private val users = app.getSystemService(UserManager::class.java)
    private val launcherApps = app.getSystemService(LauncherApps::class.java)

    fun userFor(serial: Long): UserHandle? = users.getUserForSerialNumber(serial)

    fun serialOf(user: UserHandle): Long = users.getSerialNumberForUser(user)

    fun release(id: Int) {
        try {
            host.deleteAppWidgetId(id)
        } catch (_: RuntimeException) {
            // The id is gone already.
        }
    }

    /** Providers of [user], without those that ask to stay out of the picker (`WIDGET_FEATURE_HIDE_FROM_PICKER`). */
    fun providersOf(user: UserHandle): List<AppWidgetProviderInfo> = try {
        manager.getInstalledProvidersForProfile(user).filter { (it.widgetFeatures and AppWidgetProviderInfo.WIDGET_FEATURE_HIDE_FROM_PICKER) == 0 }
    } catch (_: RuntimeException) {
        emptyList()
    }

    /** The provider [provider] of the profile [serial], also when it hides from the picker. */
    fun findProvider(provider: String, serial: Long): AppWidgetProviderInfo? {
        val user = userFor(serial) ?: return null
        val component = ComponentName.unflattenFromString(provider) ?: return null
        return try {
            manager.getInstalledProvidersForProfile(user).firstOrNull { it.provider == component }
        } catch (_: RuntimeException) {
            null
        }
    }

    /** The options a provider gets when its widget is bound: home screen category and the sizes of [span]. */
    fun options(sizes: Pair<SizeDp, SizeDp>): Bundle = Bundle().apply {
        val (portrait, landscape) = sizes
        putInt(AppWidgetManager.OPTION_APPWIDGET_HOST_CATEGORY, AppWidgetProviderInfo.WIDGET_CATEGORY_HOME_SCREEN)
        putInt(AppWidgetManager.OPTION_APPWIDGET_MIN_WIDTH, portrait.width.toInt())
        putInt(AppWidgetManager.OPTION_APPWIDGET_MIN_HEIGHT, landscape.height.toInt())
        putInt(AppWidgetManager.OPTION_APPWIDGET_MAX_WIDTH, landscape.width.toInt())
        putInt(AppWidgetManager.OPTION_APPWIDGET_MAX_HEIGHT, portrait.height.toInt())
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            putParcelableArrayList(AppWidgetManager.OPTION_APPWIDGET_SIZES, arrayListOf(SizeF(portrait.width, portrait.height), SizeF(landscape.width, landscape.height)))
        }
    }

    /** Step 1 and 2 of adding a widget (U38-7): allocate an id and bind it, with the dialog when the approval is missing. */
    fun begin(info: AppWidgetProviderInfo, span: Span, sizes: Pair<SizeDp, SizeDp>, itemId: String? = null): WidgetStep {
        val user = info.profile ?: android.os.Process.myUserHandle()
        val id = host.allocateAppWidgetId()
        val pending = PendingWidget(id, info.provider.flattenToString(), serialOf(user), span, PendingWidget.Stage.Bind, itemId)
        return bind(pending, sizes)
    }

    /** Binds the id of [pending] again for a new id (the item keeps its place) — also the second try after a refusal. */
    fun bind(pending: PendingWidget, sizes: Pair<SizeDp, SizeDp>): WidgetStep {
        val provider = ComponentName.unflattenFromString(pending.provider)
        val user = userFor(pending.userSerial)
        if (provider == null || user == null) {
            release(pending.id)
            return WidgetStep.Failed("Das Widget ist nicht verfügbar.")
        }
        val options = options(sizes)
        return if (manager.bindAppWidgetIdIfAllowed(pending.id, user, provider, options)) {
            afterBind(pending)
        } else {
            WidgetStep.AskBind(
                pending,
                Intent(AppWidgetManager.ACTION_APPWIDGET_BIND)
                    .putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, pending.id)
                    .putExtra(AppWidgetManager.EXTRA_APPWIDGET_PROVIDER, provider)
                    .putExtra(AppWidgetManager.EXTRA_APPWIDGET_PROVIDER_PROFILE, user)
                    .putExtra(AppWidgetManager.EXTRA_APPWIDGET_OPTIONS, options),
            )
        }
    }

    /** The result of the bind dialog (U38-7): `RESULT_OK` goes on, anything else frees the id. */
    fun onBindResult(pending: PendingWidget, ok: Boolean): WidgetStep {
        if (ok) return afterBind(pending)
        release(pending.id)
        return WidgetStep.Failed("Widget wurde nicht erlaubt.")
    }

    private fun afterBind(pending: PendingWidget): WidgetStep {
        if (pending.itemId != null) return WidgetStep.Finished(pending)
        val info = manager.getAppWidgetInfo(pending.id)
        return if (info != null && needsConfiguration(info)) {
            WidgetStep.AskConfigure(pending.copy(stage = PendingWidget.Stage.Configure))
        } else {
            WidgetStep.Finished(pending)
        }
    }

    /** Opens the configuration of an existing widget again: „Einrichten“ of a card and „Neu einrichten“ of the menu (U38-8). */
    fun configureStep(pending: PendingWidget): WidgetStep = WidgetStep.AskConfigure(pending)

    /** The result of a configuration activity (U38-8). A widget being added is freed on a cancel; an existing one stays. */
    fun onConfigureResult(pending: PendingWidget, ok: Boolean): WidgetStep {
        if (ok) return WidgetStep.Finished(pending)
        if (pending.itemId != null) return WidgetStep.Failed(null)
        release(pending.id)
        return WidgetStep.Failed("Einrichtung abgebrochen.")
    }

    fun needsConfiguration(info: AppWidgetProviderInfo): Boolean =
        WidgetSizes.needsConfiguration(info.configure != null, info.widgetFeatures, Build.VERSION.SDK_INT)

    fun canReconfigure(info: AppWidgetProviderInfo): Boolean = WidgetSizes.canReconfigure(info.configure != null, info.widgetFeatures)

    /**
     * Checks one widget item against the system (U38-6): the saved id still names the provider → keep it; else a new
     * id is bound without a dialog when the approval stands; else the cell asks for it; a provider that is gone is
     * removed. [profileReachable] is `false` for a locked or paused profile, whose widgets are never removed.
     */
    fun restore(item: HomeItem, sizes: Pair<SizeDp, SizeDp>, profileReachable: Boolean): Restored {
        val wanted = item.provider ?: return Restored.Gone
        val serial = item.userSerial
        val bound = item.widgetId?.let { manager.getAppWidgetInfo(it) }?.provider?.flattenToString()
        val installed = findProvider(wanted, serial) != null
        return when (WidgetRestore.classify(bound, wanted, installed, profileReachable)) {
            RestoreCase.Keep -> Restored.Keep
            RestoreCase.Remove -> Restored.Gone
            RestoreCase.Rebind -> rebind(item, wanted, serial, sizes)
        }
    }

    private fun rebind(item: HomeItem, wanted: String, serial: Long, sizes: Pair<SizeDp, SizeDp>): Restored {
        val provider = ComponentName.unflattenFromString(wanted)
        val user = userFor(serial)
        if (provider == null || user == null) return Restored.NeedsAllow
        val id = host.allocateAppWidgetId()
        if (!manager.bindAppWidgetIdIfAllowed(id, user, provider, options(sizes))) {
            release(id)
            return Restored.NeedsAllow
        }
        val info = manager.getAppWidgetInfo(id)
        return Restored.Rebound(id, configured = info == null || !needsConfiguration(info))
    }

    /**
     * The app of [info] in the profile of the provider. `AppWidgetProviderInfo.providerInfo` is hidden and
     * `getActivityInfo()` exists from API 31 only, so `LauncherApps` (API 26) answers for every profile.
     */
    fun applicationOf(info: AppWidgetProviderInfo): ApplicationInfo? = try {
        launcherApps.getApplicationInfo(info.provider.packageName, 0, info.profile)
    } catch (_: PackageManager.NameNotFoundException) {
        null
    } catch (_: SecurityException) {
        null
    }

    /** Name of the app a widget item belongs to, for the cells that wait for the person (U38-6). */
    fun appLabel(item: HomeItem): String {
        val info = item.provider?.let { findProvider(it, item.userSerial) }
        val label = info?.let { applicationOf(it) }?.loadLabel(app.packageManager)?.toString()
        return label?.takeIf { it.isNotEmpty() } ?: item.providerPackage ?: "Widget"
    }

    /** Name of a widget for the accessibility service and the menu. */
    fun label(item: HomeItem): String {
        val info = item.provider?.let { findProvider(it, item.userSerial) }
        return info?.loadLabel(app.packageManager)?.takeIf { it.isNotEmpty() } ?: appLabel(item)
    }

    companion object {
        /** The two areas of [span] in portrait and landscape (U38-9), from the grids of both orientations. */
        fun sizesOf(span: Span, portrait: Geometry, landscape: Geometry): Pair<SizeDp, SizeDp> = WidgetSizes.forSpan(span, portrait, landscape)
    }
}
