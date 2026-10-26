//! Window families use the selected existing Direct or room relationship.
use eframe::egui;
use super::screen_target::ScreenTarget;

pub(super) fn device(ui: &mut egui::Ui, profiles: &crate::share::ShareProfiles, device: &str, key: &str) {
    let mut choices = Vec::new();
    for contact in &profiles.direct_contacts {
        if contact.remote_device_id.as_deref() == Some(device)
            && contact.remote_public_key.as_deref() == Some(key) {
            choices.push((format!("Direkt: {}", contact.display_name), ScreenTarget::direct(&contact.id, Some(device))));
        }
    }
    for room in &profiles.rooms {
        for member in &room.members {
            if member.device_id == device && member.public_key == key && !member.blocked {
                choices.push((format!("Raum „{}“ / {}", room.name, member.device_name), ScreenTarget::room(&room.room_id, device)));
            }
        }
    }
    if choices.is_empty() { return; }
    ui.separator();
    let id = egui::Id::new(("family-device-relation", device, key));
    let selected = ui.ctx().data_mut(|data| data.get_temp_mut_or_insert_with(id, String::new).clone());
    let mut index = choices.iter().position(|(_, target)| target.to_peer().is_ok_and(|peer| peer.endpoint_prefix() == selected)).unwrap_or(0);
    egui::ComboBox::from_id_salt(id).selected_text(&choices[index].0).show_ui(ui, |ui| {
        for (position, (label, _)) in choices.iter().enumerate() { ui.selectable_value(&mut index, position, label); }
    });
    if let Ok(peer) = choices[index].1.to_peer() { ui.ctx().data_mut(|data| data.insert_temp(id, peer.endpoint_prefix())); }
    super::family_picker::panel(ui, &choices[index].1);
}
