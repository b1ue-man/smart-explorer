package app.smartexplorer.android.ui.vaults

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.viewmodel.compose.viewModel
import app.smartexplorer.android.R
import app.smartexplorer.android.api.VaultApi
import app.smartexplorer.android.api.VaultDetection
import app.smartexplorer.android.api.VaultSite
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.NavRequest
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.more.SubPageScaffold
import app.smartexplorer.android.ui.picker.LocationPickerDialog
import kotlinx.coroutines.launch

/** Pages below „Mehr → Tresore“; [arg] is a site ID or a job ID. */
internal enum class VaultPage { List, Create, Settings, Check, Compare }

/**
 * „Mehr → Tresore“ (VLT-M11, U31-1): the list and its sub-pages. [compareJob] opens the comparison
 * of a vault sync job (`NavRequest.OpenVaultCompare` from the job card or the notification).
 * A `locked` event of a site whose page is open returns to the list with a notice (U31-10).
 */
@Composable
fun VaultsHost(onBack: () -> Unit, compareJob: String?, onCompareOpened: () -> Unit) {
    val vm = viewModel { VaultViewModel() }
    var page by rememberSaveable { mutableStateOf(VaultPage.List) }
    var arg by rememberSaveable { mutableStateOf("") }
    var unlocking by rememberSaveable { mutableStateOf<String?>(null) }
    var syncing by rememberSaveable { mutableStateOf<String?>(null) }
    val open: (VaultPage, String) -> Unit = { target, value ->
        page = target
        arg = value
    }
    LaunchedEffect(compareJob) {
        if (compareJob != null) {
            open(VaultPage.Compare, compareJob)
            onCompareOpened()
        }
    }
    LaunchedEffect(vm) {
        vm.locked.collect { event ->
            val affected = when (page) {
                VaultPage.Settings, VaultPage.Check -> arg == event.site
                VaultPage.List, VaultPage.Create, VaultPage.Compare -> false
            }
            if (affected) {
                page = VaultPage.List
                Snackbars.show(VaultTexts.lockedNotice(vm.label(event.site), event.reason))
            }
        }
    }
    val toList = { open(VaultPage.List, "") }
    BackHandler(enabled = page != VaultPage.List, onBack = toList)
    when (page) {
        VaultPage.List -> VaultsScreen(
            vm = vm,
            onBack = onBack,
            onUnlock = { unlocking = it.site },
            onOpen = { open(it.first, it.second) },
            onSync = { syncing = it },
        )
        VaultPage.Create -> VaultCreateScreen(onBack = toList, onCreated = {
            vm.reload()
            toList()
        })
        VaultPage.Settings -> VaultSettingsScreen(vm, arg, onBack = toList, onCheck = { open(VaultPage.Check, arg) }, onSync = { syncing = arg })
        VaultPage.Check -> VaultCheckScreen(vm, arg, onBack = toList)
        VaultPage.Compare -> VaultCompareScreen(vm, arg, onBack = toList, onUnlock = { unlocking = it })
    }
    unlocking?.let { site ->
        vm.site(site)?.let { VaultUnlockSheet(it, onDismiss = { unlocking = null }, onUnlocked = { vm.reload() }) }
    }
    syncing?.let { site -> VaultSyncSheet(vm, site, onDismiss = { syncing = null }) }
}

/** The list (U31-2): one card per vault, one row per site, „Hinzufügen“ and „Alle sperren“. */
@Composable
internal fun VaultsScreen(
    vm: VaultViewModel,
    onBack: () -> Unit,
    onUnlock: (VaultSite) -> Unit,
    onOpen: (Pair<VaultPage, String>) -> Unit,
    onSync: (String) -> Unit,
) {
    val scope = rememberCoroutineScope()
    var addMenu by remember { mutableStateOf(false) }
    var picking by rememberSaveable { mutableStateOf(false) }
    var found by remember { mutableStateOf<Pair<String, VaultDetection>?>(null) }
    var removing by remember { mutableStateOf<VaultSite?>(null) }
    val add = {
        addMenu = false
        picking = true
    }
    val create = {
        addMenu = false
        onOpen(VaultPage.Create to "")
    }
    SubPageScaffold(
        title = VaultTexts.MENU_TITLE,
        onBack = onBack,
        actions = {
            if (vm.sites.any { it.unlocked }) {
                TextButton(onClick = { scope.launch { runCatchingCore { VaultApi.lockAll() } } }) { Text(VaultTexts.LOCK_ALL) }
            }
            Box {
                IconButton(onClick = { addMenu = true }) { SeIcon(R.drawable.ic_add, contentDescription = VaultTexts.ADD) }
                DropdownMenu(expanded = addMenu, onDismissRequest = { addMenu = false }) {
                    DropdownMenuItem(text = { Text(VaultTexts.ADD_EXISTING) }, onClick = add)
                    DropdownMenuItem(text = { Text(VaultTexts.CREATE_NEW) }, onClick = create)
                }
            }
        },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            LoadingBar(vm.loading)
            vm.error?.let { ErrorCard(it, Modifier.padding(16.dp)) }
            if (!vm.loading && vm.sites.isEmpty() && vm.error == null) {
                EmptyVaults(add, create)
            }
            LazyColumn(Modifier.fillMaxSize().testTag("vault-list")) {
                items(vm.groups, key = { group -> group.vaultId ?: group.sites.firstOrNull()?.site.orEmpty() }) { group ->
                    Card(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp)) {
                        group.sites.forEach { site ->
                            SiteRow(
                                site = site,
                                onTap = {
                                    when {
                                        site.unavailable -> Snackbars.show(site.stateDetail ?: VaultTexts.UNREACHABLE)
                                        site.unlocked -> AppNav.send(NavRequest.OpenLocation("vault://${site.site}/"))
                                        else -> onUnlock(site)
                                    }
                                },
                                onUnlock = { onUnlock(site) },
                                onLock = { scope.launch { runCatchingCore { VaultApi.lock(site.site) } } },
                                onOpen = onOpen,
                                onSync = { onSync(site.site) },
                                onRemove = { removing = site },
                            )
                        }
                    }
                }
            }
        }
    }
    if (picking) {
        LocationPickerDialog(
            title = VaultTexts.ADD_EXISTING,
            initialLocation = null,
            confirmLabel = VaultTexts.CHOOSE_PLACE,
            onPick = { location ->
                picking = false
                scope.launch { runCatchingCore { found = location to VaultApi.detect(location) } }
            },
            onDismiss = { picking = false },
        )
    }
    found?.let { (location, detection) ->
        EnterDialog(location, detection, onDismiss = { found = null }, onEntered = {
            found = null
            vm.reload()
        })
    }
    removing?.let { site -> RemoveDialog(site, onDone = { removing = null; vm.reload() }) }
}

@Composable
private fun EmptyVaults(onAdd: () -> Unit, onCreate: () -> Unit) {
    Column(
        Modifier.fillMaxWidth().padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(VaultTexts.EMPTY, textAlign = TextAlign.Center, style = MaterialTheme.typography.bodyMedium)
        Button(onClick = onAdd) { Text(VaultTexts.ADD_EXISTING) }
        OutlinedButton(onClick = onCreate) { Text(VaultTexts.CREATE_NEW) }
    }
}

@Composable
private fun SiteRow(
    site: VaultSite,
    onTap: () -> Unit,
    onUnlock: () -> Unit,
    onLock: () -> Unit,
    onOpen: (Pair<VaultPage, String>) -> Unit,
    onSync: () -> Unit,
    onRemove: () -> Unit,
) {
    var menu by remember { mutableStateOf(false) }
    val state = when {
        site.unavailable -> VaultTexts.UNAVAILABLE
        site.unlocked -> VaultTexts.UNLOCKED
        else -> VaultTexts.LOCKED
    }
    ListItem(
        headlineContent = { Text(site.label.ifBlank { site.site }) },
        supportingContent = {
            Column {
                Text("${placeText(site)} · $state")
                if (site.unavailable) Text(site.stateDetail ?: VaultTexts.UNREACHABLE, color = MaterialTheme.colorScheme.error)
            }
        },
        leadingContent = {
            SeIcon(if (site.unlocked) R.drawable.ic_key else R.drawable.ic_lock, contentDescription = state)
        },
        trailingContent = {
            Box {
                IconButton(onClick = { menu = true }) { SeIcon(R.drawable.ic_more_vert, contentDescription = "Weitere Aktionen") }
                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                    val item: @Composable (String, () -> Unit) -> Unit = { label, action ->
                        DropdownMenuItem(text = { Text(label) }, onClick = {
                            menu = false
                            action()
                        })
                    }
                    if (site.unlocked) item(VaultTexts.LOCK, onLock) else if (!site.unavailable) item(VaultTexts.UNLOCK, onUnlock)
                    item(VaultTexts.PROPERTIES) { onOpen(VaultPage.Settings to site.site) }
                    if (site.unlocked) item(VaultTexts.CHECK) { onOpen(VaultPage.Check to site.site) }
                    item(VaultTexts.SYNC, onSync)
                    item(VaultTexts.REMOVE, onRemove)
                }
            }
        },
        modifier = Modifier.clickable(onClick = onTap).testTag("vault-site-${site.site}"),
    )
}

internal fun placeText(site: VaultSite): String = when (site.kind) {
    "local" -> VaultTexts.LOCAL
    "share" -> VaultTexts.SHARE
    "gdrive" -> VaultTexts.GDRIVE
    else -> "Verbindung: ${site.connection ?: site.location}"
}

/** „Tresor gefunden: <Name>“ with „Bezeichnung“ and „Eintragen“ (U31-3). */
@Composable
private fun EnterDialog(location: String, detection: VaultDetection, onDismiss: () -> Unit, onEntered: () -> Unit) {
    val scope = rememberCoroutineScope()
    var label by rememberSaveable { mutableStateOf(detection.name) }
    var error by remember { mutableStateOf<String?>(null) }
    val problem = when {
        !detection.isVault -> VaultTexts.NO_VAULT
        detection.keySource == "hub" -> VaultTexts.HUB
        else -> detection.problem
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(if (problem == null) "Tresor gefunden: ${detection.name}" else VaultTexts.ADD_EXISTING) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (problem != null) {
                    Text(problem)
                } else {
                    OutlinedTextField(value = label, onValueChange = { label = it }, label = { Text(VaultTexts.LABEL) }, singleLine = true)
                }
                error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            }
        },
        confirmButton = {
            if (problem == null) {
                TextButton(onClick = {
                    scope.launch {
                        try {
                            VaultApi.add(location, label)
                            onEntered()
                        } catch (e: CoreException) {
                            error = VaultTexts.errorText(e.kind, e.message.orEmpty())
                        }
                    }
                }) { Text(VaultTexts.ENTER) }
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(VaultTexts.CANCEL) } },
    )
}

/** „Tresor austragen?“ (U31-6); unsaved drive changes ask the question of C3.3 (U31-10). */
@Composable
internal fun RemoveDialog(site: VaultSite, onDone: () -> Unit) {
    val scope = rememberCoroutineScope()
    var unsaved by remember { mutableStateOf(false) }
    if (unsaved) {
        UnsavedDialog(site.site, onDone = {
            unsaved = false
            scope.launch {
                runCatchingCore { VaultApi.remove(site.site) }
                onDone()
            }
        }, onCancel = onDone)
        return
    }
    AlertDialog(
        onDismissRequest = onDone,
        title = { Text(VaultTexts.REMOVE_TITLE) },
        text = { Text(VaultTexts.REMOVE_TEXT, Modifier.verticalScroll(rememberScrollState())) },
        confirmButton = {
            TextButton(onClick = {
                scope.launch {
                    try {
                        VaultApi.remove(site.site)
                        onDone()
                    } catch (e: CoreException) {
                        if (e.kind == "vault_unsaved_changes") unsaved = true else {
                            Snackbars.show(VaultTexts.errorText(e.kind, e.message.orEmpty(), site.label))
                            onDone()
                        }
                    }
                }
            }) { Text(VaultTexts.REMOVE_CONFIRM, color = MaterialTheme.colorScheme.error) }
        },
        dismissButton = { TextButton(onClick = onDone) { Text(VaultTexts.CANCEL) } },
    )
}

/** „Ungespeicherte Änderungen“ (C3.3, U31-10): save and lock, or keep them sealed, or cancel. */
@Composable
internal fun UnsavedDialog(site: String, onDone: () -> Unit, onCancel: () -> Unit) {
    val scope = rememberCoroutineScope()
    AlertDialog(
        onDismissRequest = onCancel,
        title = { Text(VaultTexts.UNSAVED_TITLE) },
        confirmButton = {
            Column(Modifier.verticalScroll(rememberScrollState()), horizontalAlignment = Alignment.End) {
                TextButton(onClick = { scope.launch { runCatchingCore { VaultApi.lock(site, keepUnsaved = false) }; onDone() } }) {
                    Text(VaultTexts.UNSAVED_SAVE)
                }
                TextButton(onClick = { scope.launch { runCatchingCore { VaultApi.lock(site, keepUnsaved = true) }; onDone() } }) {
                    Text(VaultTexts.UNSAVED_KEEP)
                }
                TextButton(onClick = onCancel) { Text(VaultTexts.CANCEL) }
            }
        },
    )
}

/** Runs [block]; a core error becomes a snackbar with the text of its code (U31-11). */
internal suspend fun runCatchingCore(block: suspend () -> Unit) {
    try {
        block()
    } catch (e: CoreException) {
        Snackbars.show(VaultTexts.errorText(e.kind, e.message.orEmpty()))
    }
}
