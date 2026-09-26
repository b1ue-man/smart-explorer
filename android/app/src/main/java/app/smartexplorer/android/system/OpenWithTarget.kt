package app.smartexplorer.android.system

import app.smartexplorer.android.core.VolumeInfo

/**
 * Pure part of "In Smart Explorer öffnen" (docs/refs/android-open-with.md): maps what another app
 * hands over to a folder path on a reported volume, or `null` when it must not be opened.
 */
internal object OpenWithTarget {
    /** `ExternalStorageProvider` root id of the internal (emulated) storage. */
    private const val PRIMARY_ROOT = "primary"

    /**
     * Path for an `ExternalStorageProvider` document id `root:relative/path`: `primary` is the
     * primary volume, any other root is the volume mounted as `/storage/<root>` (the FAT UUID).
     */
    fun pathForDocumentId(documentId: String, volumes: List<VolumeInfo>): String? {
        val split = documentId.indexOf(':', 1)
        if (split < 0) return null
        val root = documentId.substring(0, split)
        val relative = documentId.substring(split + 1)
        val base = if (root == PRIMARY_ROOT) {
            volumes.firstOrNull { it.primary }?.path
        } else {
            volumes.firstOrNull { it.path.trimEnd('/').substringAfterLast('/') == root }?.path
        } ?: return null
        return if (relative.isEmpty()) base else "${base.trimEnd('/')}/$relative"
    }

    /**
     * [path] normalized, if it lies on a reported volume and outside the private areas of other
     * apps (`Android/data/<app>`, `Android/obb/<app>`, which Android refuses even with all-files
     * access); `..` segments are refused rather than resolved.
     */
    fun acceptedPath(path: String, volumes: List<VolumeInfo>): String? {
        if (!path.startsWith('/') || path.contains('\u0000')) return null
        val segments = path.split('/').filter { it.isNotEmpty() && it != "." }
        if (segments.any { it == ".." }) return null
        val normalized = "/" + segments.joinToString("/")
        val volume = volumes
            .map { it.path.trimEnd('/') }
            .filter { normalized == it || normalized.startsWith("$it/") }
            .maxByOrNull { it.length } ?: return null
        val inside = normalized.removePrefix(volume).trimStart('/').split('/')
        val privateArea = inside.size >= 3 &&
            inside[0].equals("Android", ignoreCase = true) &&
            (inside[1].equals("data", ignoreCase = true) || inside[1].equals("obb", ignoreCase = true))
        return if (privateArea) null else normalized
    }
}
