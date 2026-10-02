//! Publishing side of PIN pairing (FC2): a random six-digit PIN is suggested
//! and stays readable so it can be read out, short or trivial PINs need
//! "Unsichere PIN erlauben", an offer lasts at most 30 minutes, and pairings
//! that lack the other side's confirmation are shown with "Widerrufen".

use super::super::share_discovery_state::{
    DiscoveryPinDraft, DiscoveryPublishTarget, DiscoveryUiAction, DiscoveryUiState,
};
use super::{offer_expiration_label, offer_phase_label};
use crate::app::theme;
use eframe::egui;

const MAX_OFFER_MINUTES: u64 = crate::share::DISCOVERY_MAX_OFFER_SECS / 60;

pub(super) fn publisher_ui(
    ui: &mut egui::Ui,
    state: &mut DiscoveryUiState,
    target: &DiscoveryPublishTarget,
    display_alias: &str,
    now: i64,
    actions: &mut Vec<DiscoveryUiAction>,
) {
    let active = state.offer_for_target(target).cloned();
    suggest_once(ui, state, target);
    let duration_secs = state.duration_secs();
    let pending_publish = match target {
        DiscoveryPublishTarget::Direct => state.pending_direct_publish,
        DiscoveryPublishTarget::Room { room_id, .. } => state
            .pending_room_publish
            .as_ref()
            .is_some_and(|pending| match pending {
                DiscoveryPublishTarget::Room {
                    room_id: pending_id,
                    ..
                } => pending_id == room_id,
                DiscoveryPublishTarget::Direct => false,
            }),
    };
    let allow_weak_pin = state.allow_weak_pin;
    ui.horizontal_wrapped(|ui| {
        ui.label("Dauer:");
        ui.add(
            egui::DragValue::new(&mut state.duration_minutes)
                .range(1..=MAX_OFFER_MINUTES)
                .speed(1.0)
                .suffix(" min"),
        )
        .on_hover_text("Höchstens 30 Minuten; das Angebot endet nach der ersten Kopplung.");
        ui.label("PIN:");
        let pin = pin_draft(state, target);
        ui.add(
            egui::TextEdit::singleline(pin.text_mut())
                .hint_text("6 Ziffern")
                .desired_width(100.0),
        )
        .on_hover_text("Diese PIN dem anderen Gerät nennen; sie wird nicht gespeichert.");
        if ui
            .button("Neue PIN")
            .on_hover_text("Neue zufällige sechsstellige PIN")
            .clicked()
        {
            if let Err(error) = pin.suggest() {
                state.status = Some(error);
            }
        }
        let pin = pin_draft(state, target);
        let pin_ready = pin.byte_len() <= crate::share::DISCOVERY_PIN_MAX_BYTES
            && (allow_weak_pin || pin.strength().is_acceptable());
        if let Some(offer) = active {
            let stopping = state.pending_stops.contains(&offer.offer_id);
            if ui
                .add_enabled(!stopping, egui::Button::new("Sichtbarkeit stoppen"))
                .clicked()
            {
                actions.push(DiscoveryUiAction::Stop {
                    offer_id: offer.offer_id,
                });
            }
            ui.label(offer_phase_label(offer.phase));
            ui.label(offer_expiration_label(offer.expires_at, now));
        } else if ui
            .add_enabled(
                !pending_publish && duration_secs > 0 && pin_ready,
                egui::Button::new(if pending_publish {
                    "Wird veroeffentlicht ..."
                } else {
                    "Suchbar machen"
                }),
            )
            .clicked()
        {
            actions.push(DiscoveryUiAction::Publish {
                target: target.clone(),
                display_alias: display_alias.to_string(),
                pin: crate::share::DiscoveryPin::new(pin.take()),
                duration_secs,
                allow_weak_pin,
            });
            state.allow_weak_pin = false;
        }
    });
    let weak = pin_draft(state, target).strength().problem();
    if let Some(problem) = weak {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(theme::warning(ui), problem);
            ui.checkbox(&mut state.allow_weak_pin, "Unsichere PIN erlauben")
                .on_hover_text(
                    "Kurze oder einfache PINs lassen sich durchprobieren; das Angebot endet \
                     zwar nach 5 Fehlversuchen, aber jeder Versuch ist eine Chance.",
                );
        });
    }
    pin_guidance(ui, pin_draft(state, target));
    unconfirmed_pairings_ui(ui, state, actions);
}

fn pin_draft<'a>(
    state: &'a mut DiscoveryUiState,
    target: &DiscoveryPublishTarget,
) -> &'a mut DiscoveryPinDraft {
    match target {
        DiscoveryPublishTarget::Direct => &mut state.direct_pin,
        DiscoveryPublishTarget::Room { .. } => &mut state.room_pin,
    }
}

/// Fills an empty PIN field once per target and session with a suggestion;
/// a field the user cleared afterwards stays as the user left it.
fn suggest_once(ui: &egui::Ui, state: &mut DiscoveryUiState, target: &DiscoveryPublishTarget) {
    let key = match target {
        DiscoveryPublishTarget::Direct => "direct".to_string(),
        DiscoveryPublishTarget::Room { room_id, .. } => format!("room:{room_id}"),
    };
    let id = egui::Id::new(("share_discovery_pin_suggested", key));
    if ui
        .data_mut(|data| data.get_temp::<bool>(id))
        .unwrap_or(false)
    {
        return;
    }
    ui.data_mut(|data| data.insert_temp(id, true));
    let pin = pin_draft(state, target);
    if pin.is_empty() {
        if let Err(error) = pin.suggest() {
            state.status = Some(error);
        }
    }
}

pub(super) fn pin_guidance(ui: &mut egui::Ui, pin: &DiscoveryPinDraft) {
    if pin.byte_len() > crate::share::DISCOVERY_PIN_MAX_BYTES {
        ui.colored_label(
            theme::danger(ui),
            format!(
                "PIN ist {} Bytes lang; maximal {} Bytes sind erlaubt. Es wird nichts gekuerzt.",
                pin.byte_len(),
                crate::share::DISCOVERY_PIN_MAX_BYTES
            ),
        );
    }
    ui.small("Die PIN wird als exakte UTF-8-Bytefolge verwendet und nicht dauerhaft gespeichert.");
}

/// Pairings installed without the other side's confirmation (S24).
fn unconfirmed_pairings_ui(
    ui: &mut egui::Ui,
    state: &mut DiscoveryUiState,
    actions: &mut Vec<DiscoveryUiAction>,
) {
    let mut pairings: Vec<_> = state
        .unconfirmed
        .iter()
        .map(|(exchange_id, outcome)| {
            (
                exchange_id.clone(),
                crate::share::discovery_state::unconfirmed_label(outcome),
                crate::share::discovery_state::revocable(outcome),
            )
        })
        .collect();
    pairings.sort();
    for (exchange_id, label, revocable) in pairings {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(theme::warning(ui), label);
            if revocable && ui.button("Widerrufen").clicked() {
                actions.push(DiscoveryUiAction::Revoke {
                    exchange_id: exchange_id.clone(),
                });
            }
            if ui.button("Behalten").clicked() {
                state.take_unconfirmed(&exchange_id);
            }
        });
    }
}
