package app.smartexplorer.android.launcher.se

import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.model.Kinds
import app.smartexplorer.android.launcher.ui.MenuContext
import app.smartexplorer.android.launcher.ui.MenuRow

/**
 * The rows of the menu of a se element (U40-11). A target that left the current data (`Nicht mehr vorhanden`) offers
 * only „Entfernen“. „In Dateien öffnen“ appears only when a file manager other than this app can show the folder (A36).
 */
fun seMenuRows(item: HomeItem, context: MenuContext): List<MenuRow> {
    val vm = context.vm
    val se = vm.se
    val ui = se.ui.value
    val remove = MenuRow("Entfernen") { vm.removeItem(item.id) }
    fun open(target: NavTarget) = se.open(context.activity, target)
    return when (item.kind) {
        Kinds.SE_DEVICE -> {
            val view = SeViews.device(item, ui)
            if (view.gone) return listOf(remove)
            val device = ui.data?.let { SeViews.findDevice(it, item.attr("target").orEmpty()) }
            val rows = ArrayList<MenuRow>()
            device?.screen?.let { screen ->
                rows += MenuRow("Bildschirm ansehen") { open(SeActions.openScreen(screen, control = false)) }
                rows += MenuRow("Bildschirm steuern") { open(SeActions.openScreen(screen, control = true)) }
            }
            rows += MenuRow("Geräteseite") { open(SeActions.SHARE_TAB) }
            OpenInFiles.intentFor(context.activity, ui.data?.devicesFolder, device?.key)?.let { intent ->
                rows += MenuRow("In Dateien öffnen") { se.start(context.activity, intent) }
            }
            rows += remove
            rows
        }
        Kinds.SE_ROOM -> {
            if (SeViews.room(item, ui).gone) return listOf(remove)
            val profileId = item.attr("target").orEmpty()
            listOf(MenuRow("Bildschirme des Raums") { open(SeActions.screenGrid(profileId)) }, remove)
        }
        Kinds.SE_VAULT -> {
            if (SeViews.vault(item, ui).gone) return listOf(remove)
            val site = item.attr("target").orEmpty().removePrefix("vault://").trimEnd('/')
            listOf(MenuRow("Öffnen") { open(SeActions.tapVault(site)) }, remove)
        }
        else -> listOf(remove)
    }
}
