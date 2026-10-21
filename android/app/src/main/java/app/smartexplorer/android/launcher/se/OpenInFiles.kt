package app.smartexplorer.android.launcher.se

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.provider.DocumentsContract
import app.smartexplorer.android.launcher.bridge.DevicesFolder

/**
 * „In Dateien öffnen“ (DSK-M13, U40-13, C4.8): shows the folder of a device in a file manager that is not Smart
 * Explorer. The target is the document `devices/<key>` of the documents provider. The menu entry exists only when some
 * app other than this one answers a view of the provider's root; a device without such an app shows no entry (A36).
 */
object OpenInFiles {
    /** The component Smart Explorer answers `vnd.android.document/directory` with; it is kept out of the chooser. */
    private const val OWN_ACTIVITY = "app.smartexplorer.android.MainActivity"

    /** The intent for the folder of the device [key], or `null` when the entry must not be offered. */
    fun intentFor(context: Context, folder: DevicesFolder?, key: String?): Intent? {
        if (folder == null || key == null) return null
        val probe = Intent(Intent.ACTION_VIEW).setDataAndType(
            DocumentsContract.buildRootUri(folder.authority, folder.rootId),
            DocumentsContract.Root.MIME_TYPE_ITEM,
        )
        val candidates = context.packageManager.queryIntentActivities(probe, 0).filter { it.activityInfo.packageName != context.packageName }
        if (candidates.isEmpty()) return null
        val target = Intent(Intent.ACTION_VIEW)
            .setDataAndType(DocumentsContract.buildDocumentUri(folder.authority, "${folder.rootId}/$key"), DocumentsContract.Document.MIME_TYPE_DIR)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_GRANT_READ_URI_PERMISSION)
        if (candidates.size == 1) return target.setPackage(candidates[0].activityInfo.packageName)
        // Several file managers: the system chooser, without Smart Explorer, which would only open its own folder.
        return Intent.createChooser(target, "Öffnen mit")
            .putExtra(Intent.EXTRA_EXCLUDE_COMPONENTS, arrayOf(ComponentName(context.packageName, OWN_ACTIVITY)))
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }
}
