//! Pages of the window „Geräte & Freigaben“: the tab order and the detail
//! page shown inside a tab. Tab numbers 0 to 5 keep their meaning for the
//! callers that jump to a tab (sidebar, GUI design harness, share events);
//! the pages added in 2026-10 take 6 to 8, the screen sessions (S4) 9. The detail page lives in egui
//! memory (per window, not saved), so the App state keeps its fields. Other
//! parts of the app open a device or room page with
//! `App::open_share_contact_page` and `App::open_share_room_page` (se
//! Desktop: `--open-page share-device:<id>`, `share-room:<id>`).
use std::sync::Mutex;

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
/// The screen sessions this device hosts (`share_screen_tab_ui.rs`).
pub(super) const TAB_SCREEN: usize = 9;

/// Display order: what is used most first, maintenance last.
pub(super) const TAB_ORDER: [usize; 10] = [
    TAB_DEVICES,
    TAB_REQUESTS,
    TAB_ROOMS,
    TAB_EXPORTS,
    TAB_EXEC,
    TAB_SCREEN,
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
        TAB_SCREEN => "Bildschirm",
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
    /// Pinned identity; room-only devices use the same rights page.
    Device {
        device_id: String,
        public_key: String,
    },
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

/// A page opened from outside the window; the window takes it when it draws
/// its next tab.
static PENDING_PAGE: Mutex<Option<(usize, SharePage)>> = Mutex::new(None);

fn set_pending(tab: usize, page: SharePage) {
    if let Ok(mut slot) = PENDING_PAGE.lock() {
        *slot = Some((tab, page));
    }
}

impl App {
    /// Shows the window „Geräte & Freigaben“ on the page of the saved direct
    /// contact `contact_id` (local contact id). `false` when no such contact
    /// is saved; the window then shows the overview of the tab „Geräte“.
    pub(crate) fn open_share_contact_page(&mut self, contact_id: &str) -> bool {
        let contacts = &self.share_profiles.direct_contacts;
        let known = contacts.iter().any(|contact| contact.id == contact_id);
        let page = known.then(|| SharePage::Contact(contact_id.to_string()));
        self.open_share_page(TAB_DEVICES, page)
    }

    /// Shows the window „Geräte & Freigaben“ on the page of the saved room
    /// `room_id` (local room profile id, or the room id). `false` when no such
    /// room is saved; the window then shows the overview of the tab „Räume“.
    pub(crate) fn open_share_room_page(&mut self, room_id: &str) -> bool {
        let rooms = &self.share_profiles.rooms;
        let room = rooms
            .iter()
            .find(|room| room.id == room_id || room.room_id == room_id);
        let page = room.map(|room| SharePage::Room(room.id.clone()));
        self.open_share_page(TAB_ROOMS, page)
    }

    fn open_share_page(&mut self, tab: usize, page: Option<SharePage>) -> bool {
        self.show_share = true;
        self.share_tab = tab;
        let found = page.is_some();
        set_pending(tab, page.unwrap_or(SharePage::Overview));
        found
    }
}

/// The page of `tab`; a page stored for another tab reads as its overview.
pub(super) fn current(ui: &mut egui::Ui, tab: usize) -> SharePage {
    let pending = PENDING_PAGE.lock().ok().and_then(|mut slot| slot.take());
    if let Some(pending) = pending {
        ui.data_mut(|data| data.insert_temp(page_id(), pending));
    }
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
        assert_eq!(tab_label(TAB_SCREEN), "Bildschirm");
        let mut tabs = TAB_ORDER.to_vec();
        tabs.sort_unstable();
        assert_eq!(tabs, (0..10).collect::<Vec<_>>());
        let mut labels = TAB_ORDER
            .iter()
            .map(|tab| tab_label(*tab))
            .collect::<Vec<_>>();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), TAB_ORDER.len());
    }

    fn frame(ctx: &egui::Context, seen: &mut Vec<(SharePage, SharePage)>) {
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                seen.push((current(ui, TAB_ROOMS), current(ui, TAB_DEVICES)));
            });
        });
    }

    #[test]
    fn share_pages_open_detail_pages_from_outside() {
        let ctx = egui::Context::default();
        let mut seen = Vec::new();
        set_pending(TAB_ROOMS, SharePage::Room("room-1".into()));
        frame(&ctx, &mut seen);
        // The page stays for its tab; the other tab shows its overview.
        frame(&ctx, &mut seen);
        let room = (SharePage::Room("room-1".into()), SharePage::Overview);
        assert_eq!(seen, [room.clone(), room]);
    }
}
