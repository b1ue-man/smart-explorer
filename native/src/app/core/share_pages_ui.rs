//! Pages of the window „Geräte & Freigaben“: the tab order and the detail
//! page shown inside a tab. Tab numbers 0 to 5 keep their meaning for the
//! callers that jump to a tab (sidebar, GUI design harness, share events);
//! the pages added in 2026-10 take 6 to 8. The detail page lives in egui
//! memory (per window, not saved), so the App state keeps its fields.
use super::*;

pub(super) const TAB_DEVICES: usize = 0;
pub(super) const TAB_ROOMS: usize = 1;
pub(super) const TAB_EXPORTS: usize = 2;
pub(super) const TAB_DIAGNOSTICS: usize = 3;
pub(super) const TAB_NETWORK: usize = 4;
pub(super) const TAB_TRASH: usize = 5;
pub(super) const TAB_REQUESTS: usize = 6;
pub(super) const TAB_EXEC: usize = 7;
pub(super) const TAB_THIS_DEVICE: usize = 8;

/// Display order: what is used most first, maintenance last.
pub(super) const TAB_ORDER: [usize; 9] = [
    TAB_DEVICES,
    TAB_REQUESTS,
    TAB_ROOMS,
    TAB_EXPORTS,
    TAB_EXEC,
    TAB_THIS_DEVICE,
    TAB_NETWORK,
    TAB_DIAGNOSTICS,
    TAB_TRASH,
];

pub(super) fn tab_label(tab: usize) -> &'static str {
    match tab {
        TAB_DEVICES => "Geräte",
        TAB_ROOMS => "Räume",
        TAB_EXPORTS => "Freigaben",
        TAB_NETWORK => "Netzwerk",
        TAB_TRASH => "Papierkorb",
        TAB_REQUESTS => "Anfragen",
        TAB_EXEC => "Befehle",
        TAB_THIS_DEVICE => "Dieses Gerät",
        _ => "Diagnose",
    }
}

/// The page inside a tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SharePage {
    Overview,
    /// A direct contact (local contact id).
    Contact(String),
    /// An authorization without a contact (device id).
    Grant(String),
    AddDevice,
    RemovedDevices,
    /// A saved room (local profile id).
    Room(String),
    AddRoom,
    /// The Exec card of one target (`direct/…` or `room/…` key).
    ExecTarget(String),
}

fn page_id() -> egui::Id {
    egui::Id::new("share_window_page")
}

/// The page of `tab`; a page stored for another tab reads as its overview.
pub(super) fn current(ui: &mut egui::Ui, tab: usize) -> SharePage {
    ui.data_mut(|data| data.get_temp::<(usize, SharePage)>(page_id()))
        .filter(|(stored, _)| *stored == tab)
        .map(|(_, page)| page)
        .unwrap_or(SharePage::Overview)
}

/// Shows `page` in `tab` and switches the window to that tab.
pub(super) fn open(app: &mut App, ui: &mut egui::Ui, tab: usize, page: SharePage) {
    app.share_tab = tab;
    ui.data_mut(|data| data.insert_temp(page_id(), (tab, page)));
}

/// Back to the overview of the current tab.
pub(super) fn back(ui: &mut egui::Ui) {
    ui.data_mut(|data| data.remove::<(usize, SharePage)>(page_id()));
}

/// „‹ Zurück“ and the page title; returns whether „Zurück“ was clicked.
pub(super) fn back_row(ui: &mut egui::Ui, title: &str) -> bool {
    let mut clicked = false;
    ui.horizontal_wrapped(|ui| {
        clicked = ui.button("‹ Zurück").clicked();
        ui.label(RichText::new(title).strong().size(16.0));
    });
    ui.separator();
    clicked
}

/// Small grey heading of a section on a page.
pub(super) fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).small().color(theme::muted(ui)));
}

/// Pending incoming requests (tracked and legacy) for the tab label.
pub(super) fn open_request_count(profiles: &crate::share::ShareProfiles) -> usize {
    let now = crate::share::core_now_secs();
    let (incoming, _) = super::share_lifecycle_view::request_views(profiles, now);
    let tracked = incoming.iter().filter(|request| request.can_decide).count();
    let legacy = profiles
        .legacy_direct_requests
        .iter()
        .filter(|entry| entry.is_pending(now))
        .count();
    tracked + legacy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luecken_task_share_pages_keep_old_tab_numbers_and_list_every_page_once() {
        // Sidebar and design harness jump to 0 (Geräte), 1 (Räume),
        // 2 (Freigaben) and 4 (Netzwerk).
        assert_eq!(tab_label(0), "Geräte");
        assert_eq!(tab_label(1), "Räume");
        assert_eq!(tab_label(2), "Freigaben");
        assert_eq!(tab_label(3), "Diagnose");
        assert_eq!(tab_label(4), "Netzwerk");
        assert_eq!(tab_label(5), "Papierkorb");
        let mut tabs = TAB_ORDER.to_vec();
        tabs.sort_unstable();
        assert_eq!(tabs, (0..9).collect::<Vec<_>>());
        let mut labels = TAB_ORDER
            .iter()
            .map(|tab| tab_label(*tab))
            .collect::<Vec<_>>();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), TAB_ORDER.len());
    }
}
