package app.smartexplorer.android.ui.screen

import android.content.Context
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/**
 * „Monitor“ and „Auflösung“ of the viewer (plan 2026-11-17 P5/P9, as the desktop viewer's toolbar): the [wish] for the
 * host's display (`auto`: se's own display only when the device has no monitor, `off`: the physical monitor, `request`:
 * se's own display also at a monitor) and a fixed size ([width] × [height]; `null` follows the view). The values are
 * the ones of `screen.viewOpen` and `screen.viewSetDisplay` (`virtualDisplay`, `virtualWidth`, `virtualHeight`).
 */
data class ViewerDisplayChoice(val wish: String = WISH_AUTO, val width: Int? = null, val height: Int? = null) {
    /** The fixed size as `B×H`, `null` when the display follows the view. */
    val sizeText: String? get() = if (width != null && height != null) "$width×$height" else null

    companion object {
        const val WISH_AUTO = "auto"
        const val WISH_OFF = "off"
        const val WISH_REQUEST = "request"
        val WISHES = listOf(WISH_AUTO, WISH_OFF, WISH_REQUEST)
        /** Largest side the host takes (`MAX_DISPLAY_SIDE`). */
        const val MAX_SIDE = 16384

        /** The sizes of „Auflösung“ (the desktop viewer's `SIZE_CHOICES`). */
        val SIZES = listOf(
            1280 to 720, 1280 to 800, 1360 to 768, 1440 to 900, 1600 to 900, 1680 to 1050, 1920 to 1080,
            1920 to 1200, 2560 to 1080, 2560 to 1440, 2560 to 1600, 3440 to 1440, 3840 to 2160,
        )

        /** A choice from stored values; an unknown wish is `auto`, a size needs both sides within 1..16384. */
        fun of(wish: String?, width: Int?, height: Int?): ViewerDisplayChoice {
            val valid = width != null && height != null && width in 1..MAX_SIDE && height in 1..MAX_SIDE
            return ViewerDisplayChoice(
                wish = wish?.takeIf { it in WISHES } ?: WISH_AUTO,
                width = width.takeIf { valid },
                height = height.takeIf { valid },
            )
        }

        /** `2560x1440` (also `×` or `X`, spaces around) as a size; `null` for anything else. */
        fun parseSize(text: String): Pair<Int, Int>? {
            val parts = text.trim().split('x', 'X', '×')
            if (parts.size != 2) return null
            val width = parts[0].trim().toIntOrNull() ?: return null
            val height = parts[1].trim().toIntOrNull() ?: return null
            return if (width in 1..MAX_SIDE && height in 1..MAX_SIDE) width to height else null
        }
    }
}

/**
 * The stored choice (private preferences `screen_view`, beside the codec choice) and the one in force, which
 * `screen.viewOpen` sends ([current]: read by the backend that opens the session).
 */
object ScreenDisplayPrefs {
    private const val FILE = "screen_view"
    private const val KEY_WISH = "virtual_display"
    private const val KEY_WIDTH = "virtual_width"
    private const val KEY_HEIGHT = "virtual_height"

    @Volatile
    var current: ViewerDisplayChoice = ViewerDisplayChoice()

    fun load(context: Context): ViewerDisplayChoice {
        val choice = try {
            val prefs = context.applicationContext.getSharedPreferences(FILE, Context.MODE_PRIVATE)
            ViewerDisplayChoice.of(
                prefs.getString(KEY_WISH, null),
                if (prefs.contains(KEY_WIDTH)) prefs.getInt(KEY_WIDTH, 0) else null,
                if (prefs.contains(KEY_HEIGHT)) prefs.getInt(KEY_HEIGHT, 0) else null,
            )
        } catch (e: RuntimeException) {
            ViewerDisplayChoice()
        }
        current = choice
        return choice
    }

    fun save(context: Context, choice: ViewerDisplayChoice) {
        current = choice
        try {
            context.applicationContext.getSharedPreferences(FILE, Context.MODE_PRIVATE).edit().apply {
                putString(KEY_WISH, choice.wish)
                if (choice.width != null && choice.height != null) {
                    putInt(KEY_WIDTH, choice.width)
                    putInt(KEY_HEIGHT, choice.height)
                } else {
                    remove(KEY_WIDTH)
                    remove(KEY_HEIGHT)
                }
            }.apply()
        } catch (e: RuntimeException) {
            Unit
        }
    }
}

/** The arguments of `screen.viewSetDisplay` (P9): the session and the choice as `screen.viewOpen` names it. */
object ScreenDisplayCalls {
    fun args(session: Long, choice: ViewerDisplayChoice): JsonObject = buildJsonObject {
        put("session", session)
        put("virtualDisplay", choice.wish)
        if (choice.width != null && choice.height != null) {
            put("virtualWidth", choice.width)
            put("virtualHeight", choice.height)
        }
    }
}

internal fun wishLabelRes(wish: String): Int = when (wish) {
    ViewerDisplayChoice.WISH_OFF -> R.string.screen_view_monitor_off
    ViewerDisplayChoice.WISH_REQUEST -> R.string.screen_view_monitor_request
    else -> R.string.screen_view_monitor_auto
}

/**
 * The entries „Monitor“ and „Auflösung“ of the settings menu. A change goes to [onSettings]; „Eigene Größe …“ opens
 * [DisplaySizeDialog] through [onCustom].
 */
@Composable
internal fun ColumnScope.DisplayMenuItems(settings: ViewSettings, onSettings: (ViewSettings) -> Unit, onCustom: () -> Unit) {
    val display = settings.display
    Text(
        stringResource(R.string.screen_view_display_monitor),
        style = MaterialTheme.typography.labelMedium,
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
    )
    ViewerDisplayChoice.WISHES.forEach { wish ->
        DropdownMenuItem(
            text = { Text(stringResource(wishLabelRes(wish))) },
            leadingIcon = { RadioButton(selected = display.wish == wish, onClick = null) },
            onClick = { onSettings(settings.copy(display = display.copy(wish = wish))) },
        )
    }
    Text(
        stringResource(R.string.screen_view_resolution),
        style = MaterialTheme.typography.labelMedium,
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
    )
    DropdownMenuItem(
        text = { Text(stringResource(R.string.screen_view_resolution_follow)) },
        leadingIcon = { RadioButton(selected = display.width == null, onClick = null) },
        onClick = { onSettings(settings.copy(display = display.copy(width = null, height = null))) },
    )
    ViewerDisplayChoice.SIZES.forEach { (width, height) ->
        DropdownMenuItem(
            text = { Text("$width×$height") },
            leadingIcon = { RadioButton(selected = display.width == width && display.height == height, onClick = null) },
            onClick = { onSettings(settings.copy(display = display.copy(width = width, height = height))) },
        )
    }
    val listed = ViewerDisplayChoice.SIZES.any { it.first == display.width && it.second == display.height }
    DropdownMenuItem(
        text = { Text(stringResource(R.string.screen_view_resolution_custom, display.sizeText?.takeIf { !listed } ?: "")) },
        leadingIcon = { RadioButton(selected = display.width != null && !listed, onClick = null) },
        onClick = onCustom,
    )
}

/** Asks for „Eigene Größe“ as `B×H` (for example 2560x1440); [onSet] gets the size. */
@Composable
internal fun DisplaySizeDialog(current: ViewerDisplayChoice, onDismiss: () -> Unit, onSet: (Int, Int) -> Unit) {
    var text by remember { mutableStateOf(current.width?.let { "${it}x${current.height}" } ?: "") }
    val parsed = ViewerDisplayChoice.parseSize(text)
    val invalid = text.isNotBlank() && parsed == null
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.screen_view_resolution)) },
        text = {
            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                singleLine = true,
                label = { Text(stringResource(R.string.screen_view_resolution_prompt)) },
                isError = invalid,
                supportingText = { if (invalid) Text(stringResource(R.string.screen_view_resolution_invalid)) },
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii),
            )
        },
        confirmButton = {
            TextButton(enabled = parsed != null, onClick = { parsed?.let { (width, height) -> onSet(width, height) } }) {
                Text(stringResource(R.string.screen_view_max_rate_set))
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.screen_view_max_rate_cancel)) } },
    )
}
