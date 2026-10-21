package app.smartexplorer.android.launcher.pin

import android.app.Activity
import android.appwidget.AppWidgetManager
import android.content.ActivityNotFoundException
import android.content.Intent
import android.content.pm.LauncherApps
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.drawable.Drawable
import android.os.Bundle
import android.os.UserManager
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.unit.dp
import androidx.lifecycle.lifecycleScope
import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.model.Kinds
import app.smartexplorer.android.launcher.model.LayoutOps
import app.smartexplorer.android.launcher.model.LayoutRepository
import app.smartexplorer.android.launcher.theme.LauncherTheme
import app.smartexplorer.android.launcher.widgets.LauncherWidgetHost
import app.smartexplorer.android.launcher.widgets.PendingWidget
import app.smartexplorer.android.launcher.widgets.REQ_WIDGET_BIND
import app.smartexplorer.android.launcher.widgets.REQ_WIDGET_CONFIGURE
import app.smartexplorer.android.launcher.widgets.SizeGrids
import app.smartexplorer.android.launcher.widgets.WidgetFlow
import app.smartexplorer.android.launcher.widgets.WidgetSizes
import app.smartexplorer.android.launcher.widgets.WidgetStep
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/** What the dialog shows; built from a valid request only. */
private data class PinDialogData(val title: String, val name: String, val icon: ImageBitmap?)

/**
 * The confirmation of a pin request (U38-11): an app asks the launcher to put a shortcut or a widget on the home
 * screen. The activity runs in the process `:launcher` and uses [LayoutRepository] only, because it can run while
 * `LauncherActivity` does not live. It acts on a request only when the system made it: `getPinItemRequest` answers
 * `null` for an intent an app built itself, and the activity then ends without a dialog and without an effect.
 */
class PinRequestActivity : ComponentActivity() {
    private val launcherApps by lazy { getSystemService(LauncherApps::class.java) }
    private val flow by lazy { WidgetFlow(this, LauncherWidgetHost(this)) }
    private val repository by lazy { LayoutRepository.get(this) }
    private var request: LauncherApps.PinItemRequest? = null
    private var dialog by mutableStateOf<PinDialogData?>(null)
    private var busy by mutableStateOf(false)
    private var pending: PendingWidget? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        pending = PendingWidget.decode(savedInstanceState?.getString(STATE_PENDING))
        if (!take(intent)) return
        setContent {
            LauncherTheme {
                val data = dialog
                if (data != null) {
                    AlertDialog(
                        onDismissRequest = { cancel() },
                        title = { Text(data.title) },
                        text = {
                            Row(horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                                data.icon?.let { Image(it, contentDescription = null, modifier = Modifier.size(48.dp)) }
                                Text(data.name)
                            }
                        },
                        confirmButton = { TextButton(onClick = { confirm() }, enabled = !busy) { Text("Hinzufügen") } },
                        dismissButton = { TextButton(onClick = { cancel() }, enabled = !busy) { Text("Abbrechen") } },
                    )
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        pending?.let { flow.release(it.id) }
        pending = null
        busy = false
        take(intent)
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        pending?.let { outState.putString(STATE_PENDING, it.encode()) }
    }

    /** Reads the request of [intent] and prepares the dialog; ends the activity (and returns `false`) when it is not valid. */
    private fun take(intent: Intent): Boolean {
        if (intent.action != LauncherApps.ACTION_CONFIRM_PIN_SHORTCUT && intent.action != LauncherApps.ACTION_CONFIRM_PIN_APPWIDGET) {
            finish()
            return false
        }
        val found = try {
            launcherApps.getPinItemRequest(intent)
        } catch (_: SecurityException) {
            null
        }
        if (found == null || !found.isValid) {
            finish()
            return false
        }
        request = found
        dialog = describe(found)
        if (dialog == null) {
            finish()
            return false
        }
        return true
    }

    private fun describe(found: LauncherApps.PinItemRequest): PinDialogData? {
        val dpi = resources.displayMetrics.densityDpi
        return when (found.requestType) {
            LauncherApps.PinItemRequest.REQUEST_TYPE_SHORTCUT -> {
                val info = found.shortcutInfo ?: return null
                val icon = try {
                    launcherApps.getShortcutIconDrawable(info, dpi)
                } catch (_: SecurityException) {
                    null
                }
                PinDialogData("Verknüpfung hinzufügen", info.shortLabel?.toString().orEmpty(), icon?.let(::bitmapOf))
            }
            LauncherApps.PinItemRequest.REQUEST_TYPE_APPWIDGET -> {
                val info = found.getAppWidgetProviderInfo(this) ?: return null
                PinDialogData("Widget hinzufügen", info.loadLabel(packageManager).orEmpty(), runCatching { info.loadIcon(this, dpi) }.getOrNull()?.let(::bitmapOf))
            }
            else -> null
        }
    }

    private fun bitmapOf(drawable: Drawable): ImageBitmap {
        val px = (48 * resources.displayMetrics.density).toInt().coerceAtLeast(1)
        val bitmap = Bitmap.createBitmap(px, px, Bitmap.Config.ARGB_8888)
        drawable.setBounds(0, 0, px, px)
        drawable.draw(Canvas(bitmap))
        return bitmap.asImageBitmap()
    }

    private fun cancel() {
        pending?.let { flow.release(it.id) }
        pending = null
        finish()
    }

    private fun confirm() {
        val current = request ?: return finish()
        if (!current.isValid) return finish()
        busy = true
        when (current.requestType) {
            LauncherApps.PinItemRequest.REQUEST_TYPE_SHORTCUT -> acceptShortcut(current)
            LauncherApps.PinItemRequest.REQUEST_TYPE_APPWIDGET -> startWidget(current)
            else -> finish()
        }
    }

    private fun acceptShortcut(current: LauncherApps.PinItemRequest) {
        val info = current.shortcutInfo
        if (info == null || !current.accept()) {
            end("Die Verknüpfung konnte nicht hinzugefügt werden.")
            return
        }
        val serial = getSystemService(UserManager::class.java).getSerialNumberForUser(info.userHandle)
        val attrs = JsonObject(
            mapOf(
                "component" to JsonPrimitive("${info.`package`}/"),
                "shortcutId" to JsonPrimitive(info.id),
                "user" to JsonPrimitive(serial),
            ),
        )
        place(HomeItem(id = "", kind = Kinds.SHORTCUT, attrs = attrs)) { existing ->
            existing.isShortcut && existing.packageName == info.`package` && existing.attr("shortcutId") == info.id && existing.userSerial == serial
        }
    }

    private fun startWidget(current: LauncherApps.PinItemRequest) {
        val info = current.getAppWidgetProviderInfo(this)
        if (info == null) {
            end("Das Widget konnte nicht hinzugefügt werden.")
            return
        }
        val iconDp = repository.layout.value.grid.iconDp
        val metrics = resources.displayMetrics
        val grids = SizeGrids.of(
            metrics.widthPixels / metrics.density,
            metrics.heightPixels / metrics.density - 96f,
            iconDp,
            metrics.density,
        )
        val span = WidgetSizes.initialSpan(
            targetCellWidth = if (android.os.Build.VERSION.SDK_INT >= 31) info.targetCellWidth else 0,
            targetCellHeight = if (android.os.Build.VERSION.SDK_INT >= 31) info.targetCellHeight else 0,
            minWidthPx = info.minWidth,
            minHeightPx = info.minHeight,
            geometry = grids.current,
            density = metrics.density,
        )
        run(flow.begin(info, span, WidgetSizes.forSpan(span, grids.portrait, grids.landscape)))
    }

    private fun run(step: WidgetStep) {
        when (step) {
            is WidgetStep.AskBind -> {
                pending = step.pending
                @Suppress("DEPRECATION")
                startActivityForResult(step.intent, REQ_WIDGET_BIND)
            }
            is WidgetStep.AskConfigure -> {
                pending = step.pending
                try {
                    flow.host.startAppWidgetConfigureActivityForResult(this, step.pending.id, 0, REQ_WIDGET_CONFIGURE, null)
                } catch (_: ActivityNotFoundException) {
                    run(flow.onConfigureResult(step.pending, ok = false))
                }
            }
            is WidgetStep.Finished -> acceptWidget(step.pending)
            is WidgetStep.Failed -> end(step.message ?: "Das Widget konnte nicht hinzugefügt werden.")
        }
    }

    // The configuration activity of a widget reports through `onActivityResult` (U38-7).
    @Suppress("DEPRECATION")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        val waiting = pending ?: return
        val ok = resultCode == Activity.RESULT_OK
        when (requestCode) {
            REQ_WIDGET_BIND -> run(flow.onBindResult(waiting, ok))
            REQ_WIDGET_CONFIGURE -> run(flow.onConfigureResult(waiting, ok))
        }
    }

    private fun acceptWidget(widget: PendingWidget) {
        pending = null
        val accepted = request?.accept(Bundle().apply { putInt(AppWidgetManager.EXTRA_APPWIDGET_ID, widget.id) }) == true
        if (!accepted) {
            flow.release(widget.id)
            end("Das Widget konnte nicht hinzugefügt werden.")
            return
        }
        val attrs = JsonObject(
            mapOf(
                "provider" to JsonPrimitive(widget.provider),
                "user" to JsonPrimitive(widget.userSerial),
                "widgetId" to JsonPrimitive(widget.id),
                "configured" to JsonPrimitive(true),
            ),
        )
        place(HomeItem(id = "", kind = Kinds.WIDGET, span = widget.span, attrs = attrs)) { false }
    }

    /** Puts [item] on the visible page or the next with room (the id is made here); [duplicate] names one that is there already. */
    private fun place(item: HomeItem, duplicate: (HomeItem) -> Boolean) {
        lifecycleScope.launch {
            repository.update { layout ->
                if (LayoutOps.allItems(layout).any(duplicate)) layout else LayoutOps.addAnywhere(layout, item.copy(id = LayoutOps.newId(layout)), repository.visiblePage)
            }
            repository.flush()
            finish()
        }
    }

    private fun end(message: String) {
        Toast.makeText(this, message, Toast.LENGTH_LONG).show()
        finish()
    }

    private companion object {
        const val STATE_PENDING = "pending"
    }
}
