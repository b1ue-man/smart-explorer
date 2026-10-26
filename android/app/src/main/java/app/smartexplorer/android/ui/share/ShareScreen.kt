package app.smartexplorer.android.ui.share

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import app.smartexplorer.android.R
import app.smartexplorer.android.api.ScreenApi
import app.smartexplorer.android.api.ScreenDevice
import app.smartexplorer.android.api.ShareApi
import app.smartexplorer.android.api.ShareSettingsApi
import app.smartexplorer.android.api.ShareStatus
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreEvent
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.service.ExecHostNotifier
import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.NavRequest
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.connections.RemovalReportDialog
import app.smartexplorer.android.ui.more.HintLine
import app.smartexplorer.android.ui.more.SubPageScaffold
import app.smartexplorer.android.ui.more.TextActions
import app.smartexplorer.android.ui.settings.SettingsArea
import app.smartexplorer.android.ui.settings.SettingsAreaSelector
import app.smartexplorer.android.ui.settings.ShareServerSettings
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.screen.ScreenHostSection
import app.smartexplorer.android.ui.screen.ScreenTarget
import app.smartexplorer.android.ui.screen.ScreenTargetJson
import app.smartexplorer.android.ui.screen.ScreenTilesScreen
import app.smartexplorer.android.ui.screen.ScreenViewScreen
import app.smartexplorer.android.ui.screen.toTarget
import app.smartexplorer.android.ui.transfers.TransfersSheet
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive

/**
 * Blocks of the page in spec order (C: device card → devices → rooms → requests → exports), then
 * the commands other devices may run on this phone (G4).
 */
private enum class ShareBlock { Problems, Device, Policy, Pairings, Devices, Rooms, Buttons, Requests, Rights, Exports, ExecHost, ScreenHost, Removed, DirectCode }

/**
 * Tab "Teilen" (spec F18). While visible the core polls the Share worker fast (`share.watch`);
 * `share` events reload the status. Handles [NavRequest.ShowShareRequests] by scrolling to the
 * requests.
 */
@Composable
fun ShareScreen(
    settingsArea: SettingsArea? = null,
    onSettingsAreaChange: ((SettingsArea) -> Unit)? = null,
    onBack: (() -> Unit)? = null,
) {
    val vm = viewModel { ShareViewModel() }
    val listState = rememberLazyListState()
    var dialog by remember { mutableStateOf<ShareDialog?>(null) }
    var showTransfers by rememberSaveable { mutableStateOf(false) }
    var screenDevices by remember { mutableStateOf<List<ScreenDevice>>(emptyList()) }
    var screenView by remember { mutableStateOf<Pair<ScreenTarget, String>?>(null) }
    var screenGrid by remember { mutableStateOf<ScreenGridRequest?>(null) }
    var nowMs by remember { mutableStateOf(System.currentTimeMillis()) }
    val status = vm.status
    val pending by AppNav.pending.collectAsStateWithLifecycle()
    val appContext = LocalContext.current.applicationContext

    // Runs for the whole process once started; the Share page is where commands get allowed.
    LaunchedEffect(Unit) { ExecHostNotifier.start(appContext) }

    DisposableEffect(vm) {
        onDispose { vm.setVisible(false) }
    }
    LifecycleEventEffect(Lifecycle.Event.ON_START) {
        vm.setVisible(true)
        vm.reload()
    }
    LifecycleEventEffect(Lifecycle.Event.ON_STOP) { vm.setVisible(false) }
    LaunchedEffect(vm) {
        Core.events.collect { event ->
            if (event is CoreEvent.Share || event is CoreEvent.ShareRequest) vm.reload()
        }
    }
    val offer = status?.discovery?.offer
    LaunchedEffect(offer?.offerId) {
        val until = offer?.untilMs ?: return@LaunchedEffect
        while (true) {
            nowMs = System.currentTimeMillis()
            if (nowMs >= until) {
                vm.reload()
                break
            }
            delay(1_000)
        }
    }

    // The devices the screen entries need; their online state and last probe follow the Share status.
    LaunchedEffect(status) {
        try {
            screenDevices = ScreenApi.devices()
        } catch (e: CoreException) {
            // The entries stay as they were; the next status loads again.
        }
    }
    LaunchedEffect(pending, screenDevices) {
        when (val request = pending) {
            is NavRequest.OpenApps -> {
                dialog = ShareDialog.Apps("Gerät", endpoint = request.endpoint)
                AppNav.consume(request)
            }
            is NavRequest.OpenScreen -> {
                ScreenTargetJson.parse(request.target)
                    .onSuccess { target ->
                        val name = screenDevices.firstOrNull { it.relation == target.relation && it.relationId == target.relationId && it.deviceId == target.deviceId }?.name
                        screenView = target to name.orEmpty()
                    }
                    .onFailure { Snackbars.show(it.message ?: appContext.getString(R.string.screen_view_target_invalid)) }
                AppNav.consume(request)
            }
            is NavRequest.ShowScreenGrid -> {
                screenGrid = ScreenGridRequest(request.roomId)
                AppNav.consume(request)
            }
            else -> Unit
        }
    }
    val screenLinks = ScreenLinks(
        devices = screenDevices,
        open = { device, control -> screenView = device.toTarget(control) to device.name },
        openRoom = { room -> screenGrid = ScreenGridRequest(room) },
    )

    val allBlocks = status?.let(::blocksOf) ?: listOf(ShareBlock.Problems)
    val blocks = allBlocks.filter { block ->
        settingsArea == null || block == ShareBlock.Problems || when (settingsArea) {
            SettingsArea.ThisDevice -> block in listOf(ShareBlock.Device, ShareBlock.Policy, ShareBlock.Exports)
            SettingsArea.Devices -> block in listOf(ShareBlock.Pairings, ShareBlock.Devices, ShareBlock.Buttons, ShareBlock.Requests, ShareBlock.ExecHost, ShareBlock.ScreenHost, ShareBlock.Removed, ShareBlock.DirectCode)
            SettingsArea.Rooms -> block in listOf(ShareBlock.Rooms, ShareBlock.Buttons)
            else -> false
        }
    }
    LaunchedEffect(pending, status != null, settingsArea) {
        val request = pending
        if (request != NavRequest.ShowShareRequests || status == null) return@LaunchedEffect
        if (settingsArea != null && settingsArea != SettingsArea.Devices) {
            onSettingsAreaChange?.invoke(SettingsArea.Devices)
            return@LaunchedEffect
        }
        val index = blocks.indexOf(ShareBlock.Requests)
        try {
            if (index >= 0) listState.animateScrollToItem(index)
        } catch (e: CancellationException) {
            // A touch interrupts the scroll but not this effect: the request still counts as handled.
            // A cancelled effect (new request, screen left) keeps it pending for the next run.
            if (!isActive) throw e
        }
        AppNav.consume(request)
    }

    SubPageScaffold(
        title = if (settingsArea == null) "Teilen" else "Einstellungen",
        onBack = onBack,
        actions = {
            IconButton(onClick = { vm.reload() }) { SeIcon(R.drawable.ic_refresh, contentDescription = "Aktualisieren") }
        },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            // Reloads follow every share event; only the first load and actions show the bar.
            if (settingsArea != null && onSettingsAreaChange != null) SettingsAreaSelector(settingsArea, onSettingsAreaChange)
            val incomingCount = status?.incoming.orEmpty().count { it.canAccept || it.canReject }
            if (settingsArea != null && settingsArea != SettingsArea.Devices && incomingCount > 0) {
                TextButton(onClick = { AppNav.send(NavRequest.ShowShareRequests) }, modifier = Modifier.padding(horizontal = 8.dp)) { Text("$incomingCount offene Anfragen beantworten …") }
            }
            LoadingBar(vm.busy || (vm.loading && status == null))
            LazyColumn(Modifier.fillMaxSize().testTag("share-list"), state = listState) {
                items(blocks, key = { it.name }) { block ->
                    Column {
                        ShareBlockContent(block, vm, status, nowMs, screenLinks, open = { dialog = it })
                        if (block == ShareBlock.Device && settingsArea == SettingsArea.ThisDevice) ShareServerSettings()
                    }
                }
            }
        }
    }

    dialog?.let { current ->
        key(current) {
            ShareDialogHost(
                dialog = current,
                vm = vm,
                onDismiss = { dialog = null },
                onReplace = { dialog = it },
                onShowTransfers = { showTransfers = true },
            )
        }
    }
    vm.roomCode?.let { RoomCodeDialog(it, onDismiss = { vm.roomCode = null }) }
    vm.removal?.let { RemovalReportDialog(it, onDismiss = { vm.removal = null }) }
    if (showTransfers) TransfersSheet(onDismiss = { showTransfers = false })
    screenGrid?.let { grid -> ScreenTilesScreen(grid.roomId, onClose = { screenGrid = null }) }
    screenView?.let { (target, title) -> ScreenViewScreen(target, title, onClose = { screenView = null }) }
}

/** The group view of the screens was asked for: of one room (its ID), or of all devices with `null`. */
private class ScreenGridRequest(val roomId: String?)

private fun blocksOf(status: ShareStatus): List<ShareBlock> = buildList {
    add(ShareBlock.Problems)
    add(ShareBlock.Device)
    add(ShareBlock.Policy)
    add(ShareBlock.Pairings)
    add(ShareBlock.Devices)
    add(ShareBlock.Rooms)
    add(ShareBlock.Buttons)
    if (status.incoming.isNotEmpty() || status.outgoing.isNotEmpty()) add(ShareBlock.Requests)
    add(ShareBlock.Rights)
    add(ShareBlock.Exports)
    add(ShareBlock.ExecHost)
    add(ShareBlock.ScreenHost)
    if (status.removedDevices.isNotEmpty()) add(ShareBlock.Removed)
    add(ShareBlock.DirectCode)
}

@Composable
private fun ShareBlockContent(block: ShareBlock, vm: ShareViewModel, status: ShareStatus?, nowMs: Long, screens: ScreenLinks, open: (ShareDialog) -> Unit) {
    val context = LocalContext.current
    when (block) {
        ShareBlock.Problems -> Column {
            vm.actionFailures.forEach { failure ->
                ErrorCard(
                    failure.message,
                    modifier = Modifier.padding(16.dp),
                    title = "Aktion nicht abgeschlossen",
                    actionLabel = "Erneut versuchen",
                    onAction = { if (!vm.busy) failure.retry() },
                )
                TextButton(onClick = { vm.dismissActionFailure(failure) }) { Text("Diesen Fehlerhinweis schließen") }
            }
            vm.loadError?.let { error ->
                ErrorCard(
                    error,
                    modifier = Modifier.padding(16.dp),
                    title = "Share-Status nicht geladen",
                    actionLabel = "Erneut",
                    onAction = { vm.reload() },
                )
            }
            vm.securityError?.let { error -> ErrorCard(error, modifier = Modifier.padding(16.dp), title = "Share-Sicherheit nicht geladen", actionLabel = "Erneut laden", onAction = { vm.reload() }) }
            status?.notices?.forEach { HintLine(it, Modifier.padding(top = 4.dp)) }
        }
        ShareBlock.Device -> if (status != null) {
            ThisDeviceCard(
                status,
                nowMs,
                DeviceCardActions(
                    rename = { open(ShareDialog.Rename(status.identity.deviceName)) },
                    setOnline = { online -> vm.act("Nicht geändert") { ShareApi.setOnline(online) } },
                    reconnect = { vm.act("Nicht verbunden") { ShareApi.setOnline(true) } },
                    makeDiscoverable = { open(ShareDialog.Discoverable) },
                    stopDiscoverable = { offerId -> vm.act("Nicht beendet") { ShareApi.stopDiscoverable(offerId) } },
                    copyDirectCode = { code -> TextActions.copy(context, "Direct-Code", code) },
                    setDirectOnline = { online ->
                        vm.act(
                            "Direct-Code nicht geändert",
                            if (online) "Direct-Code ist erreichbar" else "Direct-Code ist nicht erreichbar",
                        ) { ShareSettingsApi.setDirectOnline(online) }
                    },
                    renewCode = { open(ShareDialog.RenewCode) },
                    setLanPresence = { enabled -> vm.act("LAN-Einstellung nicht gespeichert") { ShareSettingsApi.setLanPresence(enabled) } },
                ),
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                serverInfo = vm.serverInfo,
            )
        }
        ShareBlock.Policy -> {
            val policy = vm.requestPolicy
            HintLine(when (policy?.requests) {
                "AutoAccept" -> "Neue Zugriffsanfragen werden automatisch angenommen."
                "Ask" -> "Neue Zugriffsanfragen warten auf deine Zustimmung."
                else -> "Anfrageeinstellung noch nicht geladen."
            })
            policy?.warning?.let { HintLine(it) }
            TextButton(onClick = { policy?.let { open(ShareDialog.RequestPolicy(it)) } }, enabled = policy != null) {
                Text(if (policy?.requests == "AutoAccept") "Immer fragen…" else "Automatisches Annehmen erlauben…")
            }
        }
        ShareBlock.Pairings -> UnconfirmedPairingsSection(vm.unconfirmed, open)
        ShareBlock.Devices -> {
            DevicesSection(status?.devices.orEmpty(), peerActions(vm, screens, open))
            DeviceRightsIndex(status, open)
        }
        ShareBlock.Rooms -> RoomsSection(status?.rooms.orEmpty(), peerActions(vm, screens, open))
        // Wraps instead of scrolling sideways, so no button hides behind the screen edge.
        ShareBlock.Buttons -> FlowRow(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            FilledTonalButton(onClick = { open(ShareDialog.Connect) }) { Text("Gerät verbinden") }
            FilledTonalButton(onClick = { open(ShareDialog.CreateRoom) }) { Text("Raum erstellen") }
            FilledTonalButton(onClick = { open(ShareDialog.JoinRoom) }) { Text("Raum beitreten") }
            FilledTonalButton(onClick = { open(ShareDialog.LoginSetup) }) { Text("Maschinenanmeldung") }
        }
        ShareBlock.Requests -> if (status != null) {
            RequestsSection(
                status,
                RequestActions(
                    decide = { request, accept ->
                        vm.act(if (accept) "Nicht angenommen" else "Nicht abgelehnt") { ShareApi.decide(request.requestId, accept) }
                    },
                    retry = { request -> vm.act("Nicht erneut gesendet", "Erneut gesendet") { ShareApi.retry(request.requestId) } },
                    delete = { request -> vm.act("Nicht gelöscht") { ShareApi.deleteRequest(request.requestId) } },
                ),
            )
        }
        ShareBlock.Exports -> if (status != null) {
            ExportsSection(
                status,
                onAdd = { scope -> open(ShareDialog.AddExport(scope)) },
                onRemove = { scope, export -> vm.act("Freigabe nicht entfernt") { ShareApi.removeExport(scope, export.path) } },
                onAccess = { scope, export, restoreHome -> open(ShareDialog.ExportAccess(scope, export, restoreHome)) },
                onConnections = { scope -> open(ShareDialog.Connections(scope)) },
                onMove = { scope, export, offset ->
                    vm.act("Reihenfolge nicht geändert") { ShareSettingsApi.moveExport(scope, export.path, offset) }
                },
            )
        }
        ShareBlock.Rights -> ContactRightsSection(status?.writeGrants, open)
        ShareBlock.ExecHost -> if (status != null) {
            ExecHostSection(
                status,
                vm.execJobs,
                ExecHostActions(
                    allow = { target -> vm.setExec(target, enabled = true) },
                    revoke = { target -> vm.setExec(target, enabled = false) },
                    stop = { job -> vm.stopExec(job) },
                    programs = { target -> open(ShareDialog.ExecPrograms(target)) },
                    log = { open(ShareDialog.ExecLog) },
                    settings = { target ->
                        val grant = status.writeGrants?.firstOrNull { it.deviceId == target.deviceId && target.relation == "direct" }
                        val member = status.rooms.flatMap { it.members }.firstOrNull { it.deviceId == target.deviceId && it.fingerprint == target.fingerprint }
                        val key = grant?.publicKey ?: member?.publicKey
                        if (key != null) open(ShareDialog.DeviceSettings(grant?.name ?: member?.name.orEmpty(), deviceId = target.deviceId, publicKey = key))
                    },
                ),
            )
        }
        ShareBlock.ScreenHost -> ScreenHostSection()
        ShareBlock.Removed -> RemovedDevicesSection(status?.removedDevices.orEmpty()) { device ->
            open(ShareDialog.Readmit(device))
        }
        ShareBlock.DirectCode -> TextButton(
            onClick = { open(ShareDialog.AddDirect) },
            modifier = Modifier.padding(start = 4.dp, bottom = 24.dp),
        ) { Text("Direct-Code hinzufügen") }
    }
}

private fun peerActions(vm: ShareViewModel, screens: ScreenLinks, open: (ShareDialog) -> Unit) = PeerActions(
    open = { location -> if (location.isNotBlank()) AppNav.send(NavRequest.OpenLocation(location)) },
    sendFiles = { target -> open(ShareDialog.SendPick(target)) },
    exec = { target -> open(ShareDialog.Exec(target)) },
    requestAccess = { device -> open(ShareDialog.RequestAccess(device.contactId, device.name)) },
    removeDevice = { device -> open(ShareDialog.RemoveDevice(device.contactId, device.name)) },
    showRoomCode = { room -> vm.showRoomCode(room.profileId, room.name) },
    leaveRoom = { room -> open(ShareDialog.LeaveRoom(room.profileId, room.name)) },
    removeRoom = { room -> open(ShareDialog.RemoveRoom(room.profileId, room.name)) },
    shareBack = { device -> open(ShareDialog.ShareBack(device)) },
    roomPolicy = { room -> open(ShareDialog.RoomPolicy(room)) },
    roomMember = { room, member, action ->
        if (action == "settings") open(ShareDialog.DeviceSettings(member.name, deviceId = member.deviceId, publicKey = member.publicKey))
        else open(ShareDialog.RoomMember(room, member, action))
    },
    connection = { device -> open(ShareDialog.DeviceSettings(device.name, contactId = device.contactId)) },
    renameRoom = { room -> open(ShareDialog.RenameRoom(room)) },
    rejoinRoom = { room -> vm.act("Nicht beigetreten", "Raum „${room.name}“ wird wieder beigetreten") { ShareSettingsApi.rejoinRoom(room.profileId) } },
    resetMemberTrust = { room, member -> open(ShareDialog.ResetMemberTrust(room, member)) },
    screens = screens,
)
