//! Window families of a device through the relation that reaches it now
//! (design 2.2, `share::peer_select::present_relation`: Direct before room);
//! no relation picker.
use super::screen_target::ScreenTarget;
use crate::share::PeerOpenTarget;
use eframe::egui;

pub(super) fn device(
    ui: &mut egui::Ui,
    profiles: &crate::share::ShareProfiles,
    device: &str,
    key: &str,
) {
    let target = match crate::share::peer_select::present_relation(profiles, device, key) {
        Some(PeerOpenTarget::Direct { contact_id }) => {
            ScreenTarget::direct(&contact_id, Some(device))
        }
        Some(PeerOpenTarget::RoomDevice { room_id, device_id }) => {
            ScreenTarget::room(&room_id, &device_id)
        }
        None => {
            ui.label(super::share_rdp_ui::OFFLINE);
            return;
        }
    };
    super::family_picker::panel(ui, &target);
}
