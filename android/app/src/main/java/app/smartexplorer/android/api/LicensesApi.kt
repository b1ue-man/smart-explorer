package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import kotlinx.serialization.Serializable

// Answer of the bridge method `app.licenses` (plan C4): the license texts of every Rust crate
// embedded in the native library, shown by "Über" → "Lizenzen der Rust-Bibliotheken".

/** One crate line of the embedded license file. */
@Serializable
data class RustCrateLicense(val name: String = "", val version: String = "", val license: String = "")

/**
 * `app.licenses`. Without an embedded file (development build) [embedded] is false, [sha256] is
 * null, [crates] is empty and [text] carries the notice "Lizenzverzeichnis fehlt in diesem
 * Entwicklungsbau".
 */
@Serializable
data class RustLicenses(
    val embedded: Boolean = false,
    val sha256: String? = null,
    val crates: List<RustCrateLicense> = emptyList(),
    val text: String = "",
)

object LicensesApi {
    suspend fun licenses(): RustLicenses = Core.request<RustLicenses>("app.licenses")
}
