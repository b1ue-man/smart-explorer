package app.smartexplorer.android.ui.vaults

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.provider.DocumentsContract
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.VaultApi
import app.smartexplorer.android.api.VaultCompareHead
import app.smartexplorer.android.api.VaultItem
import app.smartexplorer.android.api.VaultTextPair
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.system.documents.DocumentIds
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.more.SubPageScaffold
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private val IMAGE_EXTENSIONS = setOf("png", "jpg", "jpeg", "gif", "webp", "bmp")

/** The provider URI of [path] in the vault [site] (DRV-M8 IDs: `vault:<site>/<path>`). */
internal fun vaultUri(context: Context, site: String, path: String): Uri =
    DocumentsContract.buildDocumentUri(DocumentIds.authority(context), "vault:$site/${path.trim('/')}")

/**
 * A version as a bitmap, only in memory (S5): from the app's own provider, scaled so the long side
 * is at most [maxSide]; `null` when it cannot be read (vault not released to apps, locked, damaged).
 */
private fun decodeVersion(context: Context, uri: Uri, maxSide: Int): Bitmap? = try {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    context.contentResolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it, null, bounds) }
    var sample = 1
    while (maxOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= maxSide) sample *= 2
    val options = BitmapFactory.Options().apply { inSampleSize = sample }
    context.contentResolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it, null, options) }
} catch (e: Exception) {
    null
}

/**
 * One entry (U31-9): both versions, a line difference for text, both pictures for images (S5:
 * memory only, recycled on leave), else metadata with „Öffnen (A)/(B)“; the actions of its group.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun VaultCompareDetail(head: VaultCompareHead, entry: VaultItem, onBack: () -> Unit, onDone: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var texts by remember { mutableStateOf<VaultTextPair?>(null) }
    var images by remember { mutableStateOf<Pair<Bitmap?, Bitmap?>?>(null) }
    var merging by remember { mutableStateOf(false) }
    var working by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val extension = entry.path.substringAfterLast('.', "").lowercase()
    val isImage = entry.kind == "file" && extension in IMAGE_EXTENSIONS
    SecureWindow()
    LaunchedEffect(entry.item) {
        if ("merge" in entry.actions) {
            try {
                texts = VaultApi.compareText(head.job, entry.item)
            } catch (e: CoreException) {
                error = VaultTexts.errorText(e.kind, e.message.orEmpty())
            }
        } else if (isImage) {
            val side = context.resources.displayMetrics.widthPixels
            images = withContext(Dispatchers.IO) {
                val a = if (entry.a.exists) decodeVersion(context, vaultUri(context, head.siteA, entry.path), side) else null
                val b = if (entry.b.exists) decodeVersion(context, vaultUri(context, head.siteB, entry.path), side) else null
                a to b
            }
        }
    }
    DisposableEffect(entry.item) {
        onDispose {
            images?.first?.recycle()
            images?.second?.recycle()
            texts = null
        }
    }
    if (merging && texts != null) {
        VaultMergeRows(texts!!, onBack = { merging = false }, onApply = { content ->
            scope.launch {
                working = true
                try {
                    VaultApi.mergeApply(head.job, entry.item, content)
                    onDone()
                } catch (e: CoreException) {
                    error = VaultTexts.errorText(e.kind, e.message.orEmpty())
                    merging = false
                } finally {
                    working = false
                }
            }
        })
        return
    }
    SubPageScaffold(title = entry.path.substringAfterLast('/').ifBlank { VaultTexts.COMPARE }, onBack = onBack) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).padding(16.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(entry.path, style = MaterialTheme.typography.titleSmall)
            error?.let { ErrorCard(it) }
            Text("${head.labelA}: ${sideText(entry.a.exists, entry.a.size, entry.a.mtimeMs)}")
            Text("${head.labelB}: ${sideText(entry.b.exists, entry.b.size, entry.b.mtimeMs)}")
            texts?.let { pair -> DiffRows(pair) }
            images?.let { (a, b) ->
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    listOf(a, b).forEach { picture ->
                        Column(Modifier.weight(1f)) {
                            if (picture != null) {
                                Image(picture.asImageBitmap(), contentDescription = null, modifier = Modifier.fillMaxWidth().heightIn(max = 320.dp))
                            } else {
                                Text(VaultTexts.NO_PREVIEW)
                            }
                        }
                    }
                }
            }
            if (texts == null && !isImage) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (entry.a.exists) OutlinedButton(onClick = { openVersion(context, head.siteA, entry.path) }) { Text("Öffnen (A)") }
                    if (entry.b.exists) OutlinedButton(onClick = { openVersion(context, head.siteB, entry.path) }) { Text("Öffnen (B)") }
                }
            }
            if (working) LinearProgressIndicator(Modifier.fillMaxWidth())
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                entry.actions.forEach { action ->
                    if (action == "merge") {
                        if (texts != null) OutlinedButton(enabled = !working, onClick = { merging = true }) { Text(VaultTexts.MERGE) }
                    } else {
                        Button(enabled = !working, onClick = {
                            scope.launch {
                                working = true
                                try {
                                    VaultApi.resolve(head.job, entry.item, action, confirmed = true)
                                    onDone()
                                } catch (e: CoreException) {
                                    error = VaultTexts.errorText(e.kind, e.message.orEmpty())
                                } finally {
                                    working = false
                                }
                            }
                        }) { Text(VaultTexts.actionTitle(action)) }
                    }
                }
                OutlinedButton(onClick = onBack) { Text(VaultTexts.SKIP) }
            }
            if ("keep-both" in entry.actions) Text(VaultTexts.KEEP_BOTH_HINT, style = MaterialTheme.typography.bodySmall)
        }
    }
}

/** The line difference: „+“/„−“ marks and colors, so it does not hang on color alone. Not selectable (S7). */
@Composable
private fun DiffRows(pair: VaultTextPair) {
    Column {
        pair.rows.forEach { row ->
            if (row.equal) {
                DiffLine("  ", row.left.orEmpty(), null)
            } else {
                row.left?.let { DiffLine("− ", it, MaterialTheme.colorScheme.errorContainer) }
                row.right?.let { DiffLine("+ ", it, MaterialTheme.colorScheme.tertiaryContainer) }
            }
        }
    }
}

@Composable
private fun DiffLine(mark: String, text: String, color: androidx.compose.ui.graphics.Color?) {
    val modifier = if (color != null) Modifier.fillMaxWidth().background(color) else Modifier.fillMaxWidth()
    Text(mark + text, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall, modifier = modifier)
}

private fun openVersion(context: Context, site: String, path: String) {
    val intent = Intent(Intent.ACTION_VIEW, vaultUri(context, site, path)).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
    try {
        context.startActivity(intent)
    } catch (e: ActivityNotFoundException) {
        Snackbars.show("Keine App zum Öffnen gefunden.")
    } catch (e: SecurityException) {
        Snackbars.show(VaultTexts.errorText("vault_use_provider", ""))
    }
}
