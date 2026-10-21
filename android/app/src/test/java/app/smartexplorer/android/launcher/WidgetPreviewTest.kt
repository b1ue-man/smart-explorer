package app.smartexplorer.android.launcher

import app.smartexplorer.android.launcher.widgets.PreviewKind
import app.smartexplorer.android.launcher.widgets.previewKind
import org.junit.Assert.assertEquals
import org.junit.Test

/** The order of widget previews (U38-10, P10), without Android. */
class WidgetPreviewTest {
    @Test
    fun launcher_task_widget_picker_preview_order() {
        // API 35 with the home screen category: generated, with or without a preview layout.
        assertEquals(PreviewKind.Generated, previewKind(35, true, 5))
        assertEquals(PreviewKind.Generated, previewKind(35, true, 0))
        // API 35 without the category, and API 31 with a layout: the layout.
        assertEquals(PreviewKind.Layout, previewKind(35, false, 5))
        assertEquals(PreviewKind.Layout, previewKind(31, false, 5))
        // API 31 without a layout, and API 30 whatever the layout field says: the image.
        assertEquals(PreviewKind.Image, previewKind(31, false, 0))
        assertEquals(PreviewKind.Image, previewKind(30, false, 5))
    }

    @Test
    fun theCategoryDoesNothingBeforeApi35() {
        assertEquals(PreviewKind.Layout, previewKind(34, true, 5))
        assertEquals(PreviewKind.Image, previewKind(34, true, 0))
        assertEquals(PreviewKind.Image, previewKind(30, true, 0))
        assertEquals(PreviewKind.Image, previewKind(30, true, 5))
    }

    @Test
    fun aGeneratorThatAnswersNothingFallsBackBySameOrderWithoutTheCategory() {
        // What `load` does after `getWidgetPreview` returned null (U38-10): the same function with the category off.
        assertEquals(PreviewKind.Layout, previewKind(36, false, 7))
        assertEquals(PreviewKind.Image, previewKind(36, false, 0))
    }
}
