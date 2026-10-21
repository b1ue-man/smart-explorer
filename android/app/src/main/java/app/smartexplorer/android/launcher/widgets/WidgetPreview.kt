package app.smartexplorer.android.launcher.widgets

import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProviderInfo
import android.content.Context
import android.graphics.drawable.Drawable
import android.os.Build
import android.os.UserHandle
import android.view.LayoutInflater
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.RemoteViews
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Modifier
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** How a widget's preview is made (U38-10). */
enum class PreviewKind { Generated, Layout, Image }

/**
 * The order of the previews (P10, U38-10), pure: from API 35 a preview generated for the home screen
 * category, from API 31 the preview layout, else the preview image (or the provider's icon).
 */
fun previewKind(sdk: Int, hasHomeScreenCategory: Boolean, previewLayout: Int): PreviewKind = when {
    sdk >= 35 && hasHomeScreenCategory -> PreviewKind.Generated
    sdk >= 31 && previewLayout != 0 -> PreviewKind.Layout
    else -> PreviewKind.Image
}

/** What was loaded for a preview; drawn on the main thread by [WidgetPreview]. */
private sealed interface PreviewContent {
    class Remote(val views: RemoteViews) : PreviewContent

    class LayoutRes(val context: Context, val layout: Int) : PreviewContent

    class Picture(val drawable: Drawable?) : PreviewContent
}

private fun hasHomeScreenCategory(info: AppWidgetProviderInfo): Boolean =
    Build.VERSION.SDK_INT >= 35 && (info.generatedPreviewCategories and AppWidgetProviderInfo.WIDGET_CATEGORY_HOME_SCREEN) != 0

private fun previewLayoutOf(info: AppWidgetProviderInfo): Int = if (Build.VERSION.SDK_INT >= 31) info.previewLayout else 0

private fun load(context: Context, manager: AppWidgetManager, info: AppWidgetProviderInfo, user: UserHandle, densityDpi: Int): PreviewContent {
    val sdk = Build.VERSION.SDK_INT
    var kind = previewKind(sdk, hasHomeScreenCategory(info), previewLayoutOf(info))
    if (kind == PreviewKind.Generated) {
        // A provider that generates nothing answers `null`: the same order again without the category.
        val views = try {
            manager.getWidgetPreview(info.provider, user, AppWidgetProviderInfo.WIDGET_CATEGORY_HOME_SCREEN)
        } catch (_: RuntimeException) {
            null
        }
        if (views != null) return PreviewContent.Remote(views)
        kind = previewKind(sdk, false, previewLayoutOf(info))
    }
    if (kind == PreviewKind.Layout) {
        // The layout belongs to the provider's package, so it is inflated with that package's resources; a package
        // of another profile cannot be reached from here and falls back to the image.
        val providerContext = try {
            context.createPackageContext(info.provider.packageName, 0)
        } catch (_: Exception) {
            null
        }
        if (providerContext != null) return PreviewContent.LayoutRes(providerContext, previewLayoutOf(info))
    }
    val picture = try {
        info.loadPreviewImage(context, densityDpi) ?: info.loadIcon(context, densityDpi)
    } catch (_: RuntimeException) {
        null
    }
    return PreviewContent.Picture(picture)
}

/** The preview of a provider: loaded off the main thread, shown in the picker (U38-10). */
@Composable
fun WidgetPreview(info: AppWidgetProviderInfo, user: UserHandle, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val manager = AppWidgetManager.getInstance(context)
    val densityDpi = context.resources.displayMetrics.densityDpi
    val content by produceState<PreviewContent?>(null, info.provider, user) {
        value = withContext(Dispatchers.IO) { load(context, manager, info, user, densityDpi) }
    }
    val loaded = content ?: return
    AndroidView(
        factory = { ctx -> FrameLayout(ctx) },
        modifier = modifier,
        update = { frame ->
            frame.removeAllViews()
            try {
                when (loaded) {
                    is PreviewContent.Remote -> frame.addView(loaded.views.apply(frame.context, frame))
                    is PreviewContent.LayoutRes -> frame.addView(LayoutInflater.from(loaded.context).inflate(loaded.layout, frame, false))
                    is PreviewContent.Picture -> frame.addView(
                        ImageView(frame.context).apply {
                            scaleType = ImageView.ScaleType.FIT_CENTER
                            setImageDrawable(loaded.drawable)
                        },
                    )
                }
            } catch (_: RuntimeException) {
                // A preview that cannot be drawn stays empty; the entry still has its name and button.
            }
        },
    )
}
