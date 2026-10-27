//! Window families use the selected existing Direct or room relationship.
use super::screen_target::ScreenTarget;
use eframe::egui;

pub(super) fn device(
    ui: &mut egui::Ui,
    profiles: &crate::share::ShareProfiles,
    device: &str,
    key: &str,
) {
    let mut choices = Vec::new();
    for contact in &profiles.direct_contacts {
        if contact.remote_device_id.as_deref() == Some(device)
            && contact.remote_public_key.as_deref() == Some(key)
        {
            choices.push((
                format!("Direkt: {}", contact.display_name),
                ScreenTarget::direct(&contact.id, Some(device)),
            ));
        }
    }
    for room in &profiles.rooms {
        for member in &room.members {
            if member.device_id == device && member.public_key == key && !member.blocked {
                choices.push((
                    format!("Raum „{}“ / {}", room.name, member.device_name),
                    ScreenTarget::room(&room.room_id, device),
                ));
            }
        }
    }
    if choices.is_empty() {
        return;
    }
    let id = egui::Id::new(("family-device-relation", device, key));
    let selected = ui
        .ctx()
        .data_mut(|data| data.get_temp_mut_or_insert_with(id, String::new).clone());
    let mut index = choices
        .iter()
        .position(|(_, target)| {
            target
                .to_peer()
                .is_ok_and(|peer| peer.endpoint_prefix() == selected)
        })
        .unwrap_or(0);
    let labels: Vec<&str> = choices.iter().map(|(label, _)| label.as_str()).collect();
    super::device_page_layout::relation_picker(ui, &labels, &mut index);
    if let Ok(peer) = choices[index].1.to_peer() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(id, peer.endpoint_prefix()));
    }
    super::family_picker::panel(ui, &choices[index].1);
}
