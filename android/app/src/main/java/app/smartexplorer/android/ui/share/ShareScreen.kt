package app.smartexplorer.android.ui.share

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import app.smartexplorer.android.R
import app.smartexplorer.android.api.ShareApi
import app.smartexplorer.android.api.ShareStatus
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreEvent
import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.NavRequest
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.connections.RemovalReportDialog
import app.smartexplorer.android.ui.more.HintLine
import app.smartexplorer.android.ui.more.SubPageScaffold
import app.smartexplorer.android.ui.more.TextActions
import app.smartexplorer.android.ui.transfers.TransfersSheet
import kotlinx.coroutines.delay

/** Blocks of the page in spec order (C: device card → devices → rooms → requests → exports). */
private enum class ShareBlock { Problems, Device, Devices, Rooms, Buttons, Requests, Exports, Removed, DirectCode }

/**
 * Tab "Teilen" (spec F18). While visible the core polls the Share worker fast (`share.watch`);
 * `share` events reload the status. Handles [NavRequest.ShowShareRequests] by scrolling to the
 * requests.
 */
@Composable
fun ShareScreen() {
    val vm = viewModel { ShareViewModel() }
    val listState = rememberLazyListState()
    var dialog by remember { mutableStateOf<ShareDialog?>(null) }
    var showTransfers by rememberSaveable { mutableStateOf(false) }
    var nowMs by remember { mutableStateOf(System.currentTimeMillis()) }
    val status = vm.status
    val pending by AppNav.pending.collectAsStateWithLifecycle()

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

    val blocks = status?.let(::blocksOf) ?: listOf(ShareBlock.Problems)
    LaunchedEffect(pending, status != null) {
        val request = pending
        if (request != NavRequest.ShowShareRequests || status == null) return@LaunchedEffect
        val index = blocks.indexOf(ShareBlock.Requests)
        if (index >= 0) listState.animateScrollToItem(index)
        AppNav.consume(request)
    }

    SubPageScaffold(
        title = "Teilen",
        onBack = null,
        actions = {
            IconButton(onClick = { vm.reload() }) { SeIcon(R.drawable.ic_refresh, contentDescription = "Aktualisieren") }
        },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            // Reloads follow every share event; only the first load and actions show the bar.
            LoadingBar(vm.busy || (vm.loading && status == null))
            LazyColumn(Modifier.fillMaxSize(), state = listState) {
                items(blocks, key = { it.name }) { block ->
                    Column { ShareBlockContent(block, vm, status, nowMs, open = { dialog = it }) }
                }
            }
        }
    }

    dialog?.let { current ->
        ShareDialogHost(
            dialog = current,
            vm = vm,
            onDismiss = { dialog = null },
            onReplace = { dialog = it },
            onShowTransfers = { showTransfers = true },
        )
    }
    vm.roomCode?.let { RoomCodeDialog(it, onDismiss = { vm.roomCode = null }) }
    vm.removal?.let { RemovalReportDialog(it, onDismiss = { vm.removal = null }) }
    if (showTransfers) TransfersSheet(onDismiss = { showTransfers = false })
}

private fun blocksOf(status: ShareStatus): List<ShareBlock> = buildList {
    add(ShareBlock.Problems)
    add(ShareBlock.Device)
    add(ShareBlock.Devices)
    add(ShareBlock.Rooms)
    add(ShareBlock.Buttons)
    if (status.incoming.isNotEmpty() || status.outgoing.isNotEmpty()) add(ShareBlock.Requests)
    add(ShareBlock.Exports)
    if (status.removedDevices.isNotEmpty()) add(ShareBlock.Removed)
    add(ShareBlock.DirectCode)
}

@Composable
private fun ShareBlockContent(block: ShareBlock, vm: ShareViewModel, status: ShareStatus?, nowMs: Long, open: (ShareDialog) -> Unit) {
    val context = LocalContext.current
    when (block) {
        ShareBlock.Problems -> Column {
            vm.loadError?.let { error ->
                ErrorCard(
                    error,
                    modifier = Modifier.padding(16.dp),
                    title = "Share-Status nicht geladen",
                    actionLabel = "Erneut",
                    onAction = { vm.reload() },
                )
            }
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
                ),
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            )
        }
        ShareBlock.Devices -> DevicesSection(status?.devices.orEmpty(), peerActions(vm, open))
        ShareBlock.Rooms -> RoomsSection(status?.rooms.orEmpty(), peerActions(vm, open))
        ShareBlock.Buttons -> Row(
            Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            FilledTonalButton(onClick = { open(ShareDialog.Connect) }) { Text("Gerät verbinden") }
            FilledTonalButton(onClick = { open(ShareDialog.CreateRoom) }) { Text("Raum erstellen") }
            FilledTonalButton(onClick = { open(ShareDialog.JoinRoom) }) { Text("Raum beitreten") }
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
            )
        }
        ShareBlock.Removed -> RemovedDevicesSection(status?.removedDevices.orEmpty()) { device ->
            vm.act("Nicht zugelassen", "Wieder zugelassen: ${device.name}") { ShareApi.readmit(device.deviceId) }
        }
        ShareBlock.DirectCode -> TextButton(
            onClick = { open(ShareDialog.AddDirect) },
            modifier = Modifier.padding(start = 4.dp, bottom = 24.dp),
        ) { Text("Direct-Code hinzufügen") }
    }
}

private fun peerActions(vm: ShareViewModel, open: (ShareDialog) -> Unit) = PeerActions(
    open = { location -> if (location.isNotBlank()) AppNav.send(NavRequest.OpenLocation(location)) },
    sendFiles = { target -> open(ShareDialog.SendPick(target)) },
    exec = { target -> open(ShareDialog.Exec(target)) },
    requestAccess = { device -> open(ShareDialog.RequestAccess(device.contactId, device.name)) },
    removeDevice = { device -> open(ShareDialog.RemoveDevice(device.contactId, device.name)) },
    showRoomCode = { room -> vm.showRoomCode(room.profileId, room.name) },
    leaveRoom = { room -> open(ShareDialog.LeaveRoom(room.profileId, room.name)) },
    removeRoom = { room -> open(ShareDialog.RemoveRoom(room.profileId, room.name)) },
)
