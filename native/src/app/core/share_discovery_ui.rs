use super::share_discovery_state::{
    DiscoveryExchangeState, DiscoveryListEntry, DiscoveryOfferPhase, DiscoveryPublishTarget,
    DiscoveryUiAction, DiscoveryUiKind,
};
use super::App;

impl App {
    pub(in crate::app) fn ui_share_discovery_direct(&mut self, ui: &mut egui::Ui) {
        let now = unix_now();
        self.share_discovery.prune_expired(now);
        let alias = self.share_device_draft.clone();
        let target = DiscoveryPublishTarget::Direct;
        let mut actions = Vec::new();
        queue_initial_refresh(&mut self.share_discovery, &mut actions);

        discovery_heading(ui, "AUFFINDBARKEIT");
        publisher_ui(
            ui,
            &mut self.share_discovery,
            &target,
            &alias,
            now,
            &mut actions,
        );
        ui.add_space(4.0);
        discovery_list_ui(
            ui,
            &mut self.share_discovery,
            DiscoveryUiKind::Direct,
            now,
            &mut actions,
        );
        for action in actions {
            self.dispatch_discovery_ui_action(action, ui.ctx().clone());
        }
    }

    pub(in crate::app) fn ui_share_discovery_rooms(&mut self, ui: &mut egui::Ui) {
        let now = unix_now();
        self.share_discovery.prune_expired(now);
        let rooms: Vec<(String, String)> = self
            .share_profiles
            .rooms
            .iter()
            .map(|room| (room.id.clone(), room.name.clone()))
            .collect();
        if !rooms
            .iter()
            .any(|(id, _)| id == &self.share_discovery.selected_room_id)
        {
            self.share_discovery.selected_room_id = rooms
                .first()
                .map(|(id, _)| id.clone())
                .unwrap_or_default();
        }
        let selected = rooms
            .iter()
            .find(|(id, _)| id == &self.share_discovery.selected_room_id)
            .cloned();
        let mut actions = Vec::new();
        queue_initial_refresh(&mut self.share_discovery, &mut actions);

        discovery_heading(ui, "RAUM-AUFFINDBARKEIT");
        if rooms.is_empty() {
            ui.label("Erstelle oder speichere zuerst einen Raum.");
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.label("Raum:");
                let selected_name = selected
                    .as_ref()
                    .map(|(_, name)| name.as_str())
                    .unwrap_or("Raum auswaehlen");
                egui::ComboBox::from_id_salt("share_discovery_room")
                    .selected_text(selected_name)
                    .show_ui(ui, |ui| {
                        for (id, name) in &rooms {
                            ui.selectable_value(
                                &mut self.share_discovery.selected_room_id,
                                id.clone(),
                                name,
                            );
                        }
                    });
            });
            let selected = rooms
                .iter()
                .find(|(id, _)| id == &self.share_discovery.selected_room_id)
                .cloned();
            if let Some((room_id, room_name)) = selected {
                let target = DiscoveryPublishTarget::Room {
                    room_id,
                    room_name: room_name.clone(),
                };
                publisher_ui(
                    ui,
                    &mut self.share_discovery,
                    &target,
                    &room_name,
                    now,
                    &mut actions,
                );
            }
        }
        active_room_offers_ui(
            ui,
            &self.share_discovery,
            &self.share_discovery.selected_room_id,
            now,
            &mut actions,
        );
        ui.add_space(4.0);
        discovery_list_ui(
            ui,
            &mut self.share_discovery,
            DiscoveryUiKind::Room,
            now,
            &mut actions,
        );
        for action in actions {
            self.dispatch_discovery_ui_action(action, ui.ctx().clone());
        }
    }
}

fn discovery_heading(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .small()
            .color(egui::Color32::from_gray(140)),
    );
}

fn queue_initial_refresh(
    state: &mut super::share_discovery_state::DiscoveryUiState,
    actions: &mut Vec<DiscoveryUiAction>,
) {
    if !state.initial_refresh_requested {
        state.initial_refresh_requested = true;
        actions.push(DiscoveryUiAction::Refresh);
    }
}

fn active_room_offers_ui(
    ui: &mut egui::Ui,
    state: &super::share_discovery_state::DiscoveryUiState,
    selected_room_id: &str,
    now: i64,
    actions: &mut Vec<DiscoveryUiAction>,
) {
    let other_offers: Vec<_> = state
        .active_offers
        .iter()
        .filter_map(|offer| match &offer.target {
            DiscoveryPublishTarget::Room {
                room_id,
                room_name,
            } if room_id != selected_room_id => Some((
                offer.offer_id.clone(),
                room_name.clone(),
                offer.expires_at,
                offer.phase,
            )),
            DiscoveryPublishTarget::Room { .. } | DiscoveryPublishTarget::Direct => None,
        })
        .collect();
    for (offer_id, room_name, expires_at, phase) in other_offers {
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("Weiterer sichtbarer Raum: {room_name}"));
            ui.label(offer_phase_label(phase));
            ui.label(offer_expiration_label(expires_at, now));
            let stopping = state.pending_stops.contains(&offer_id);
            if ui
                .add_enabled(!stopping, egui::Button::new("Sichtbarkeit stoppen"))
                .clicked()
            {
                actions.push(DiscoveryUiAction::Stop { offer_id });
            }
        });
    }
}

fn publisher_ui(
    ui: &mut egui::Ui,
    state: &mut super::share_discovery_state::DiscoveryUiState,
    target: &DiscoveryPublishTarget,
    display_alias: &str,
    now: i64,
    actions: &mut Vec<DiscoveryUiAction>,
) {
    let active = state.offer_for_target(target).cloned();
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
    ui.horizontal_wrapped(|ui| {
        ui.label("Dauer:");
        ui.add(
            egui::DragValue::new(&mut state.duration_minutes)
                .speed(1.0)
                .suffix(" min"),
        );
        ui.label("PIN:");
        let pin = match target {
            DiscoveryPublishTarget::Direct => &mut state.direct_pin,
            DiscoveryPublishTarget::Room { .. } => &mut state.room_pin,
        };
        ui.add(
            egui::TextEdit::singleline(pin.text_mut())
                .password(true)
                .hint_text("frei waehlbar")
                .desired_width(140.0),
        );
        let pin_bytes = pin.byte_len();
        let pin_valid = pin_bytes <= crate::share::DISCOVERY_PIN_MAX_BYTES;
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
                !pending_publish && duration_secs > 0 && pin_valid,
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
            });
        }
    });
    if duration_secs == 0 {
        ui.colored_label(
            egui::Color32::from_rgb(220, 100, 90),
            "Die Sichtbarkeitsdauer muss groesser als 0 sein.",
        );
    }
    let pin = match target {
        DiscoveryPublishTarget::Direct => &state.direct_pin,
        DiscoveryPublishTarget::Room { .. } => &state.room_pin,
    };
    pin_guidance(ui, pin);
}

fn discovery_list_ui(
    ui: &mut egui::Ui,
    state: &mut super::share_discovery_state::DiscoveryUiState,
    kind: DiscoveryUiKind,
    now: i64,
    actions: &mut Vec<DiscoveryUiAction>,
) {
    ui.horizontal_wrapped(|ui| {
        discovery_heading(ui, "AUFFINDBARE ZIELE");
        if ui
            .add_enabled(!state.refreshing, egui::Button::new("Liste aktualisieren"))
            .clicked()
        {
            actions.push(DiscoveryUiAction::Refresh);
        }
        if state.refreshing {
            ui.spinner();
        }
    });

    let entries: Vec<DiscoveryListEntry> = state
        .entries
        .iter()
        .filter(|entry| entry.kind == kind)
        .cloned()
        .collect();
    if entries.is_empty() {
        ui.label(format!("Keine auffindbaren {}e.", kind.label()));
    }
    for entry in entries {
        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong(format!("Ungepruefter Anzeigename: {}", entry.display_alias));
                ui.label(expiration_label(entry.expires_at, now));
                let compatible = entry.compatibility.can_connect();
                let color = if compatible {
                    egui::Color32::from_rgb(100, 190, 120)
                } else {
                    egui::Color32::from_rgb(240, 160, 100)
                };
                ui.colored_label(color, entry.compatibility.label());
            });
            let exchange = state
                .exchange_for_discovery(&entry.discovery_id)
                .map(|(exchange_id, record)| {
                    (exchange_id.to_string(), record.state.clone())
                });
            let starting = state.starting(&entry.discovery_id);
            let pending = starting
                || exchange
                    .as_ref()
                    .is_some_and(|(_, exchange)| exchange.is_pending());
            ui.horizontal_wrapped(|ui| {
                ui.label("PIN:");
                let pin = state
                    .entry_pins
                    .entry(entry.discovery_id.clone())
                    .or_default();
                ui.add(
                    egui::TextEdit::singleline(pin.text_mut())
                        .password(true)
                        .hint_text("exakte PIN")
                        .desired_width(140.0),
                );
                let pin_valid = pin.byte_len() <= crate::share::DISCOVERY_PIN_MAX_BYTES;
                if ui
                    .add_enabled(
                        entry.compatibility.can_connect() && !pending && pin_valid,
                        egui::Button::new(if pending { "Exchange laeuft ..." } else { "Connect" }),
                    )
                    .clicked()
                {
                    actions.push(DiscoveryUiAction::Connect {
                        discovery_id: entry.discovery_id.clone(),
                        pin: crate::share::DiscoveryPin::new(pin.take()),
                    });
                }
            });
            if let Some(pin) = state.entry_pins.get(&entry.discovery_id) {
                pin_guidance(ui, pin);
            }
            if starting {
                ui.small("Verbindung wird im Hintergrund gestartet");
            }
            if let Some((exchange_id, exchange)) = exchange {
                ui.horizontal_wrapped(|ui| {
                    ui.small(exchange.label());
                    match exchange {
                        DiscoveryExchangeState::Exchanging => {
                            if ui.button("Abbrechen").clicked() {
                                actions.push(DiscoveryUiAction::Cancel { exchange_id });
                            }
                        }
                        DiscoveryExchangeState::Cancelling => {
                            ui.add_enabled(false, egui::Button::new("Abbruch laeuft ..."));
                        }
                        DiscoveryExchangeState::Cancelled
                        | DiscoveryExchangeState::Complete(_)
                        | DiscoveryExchangeState::Failed(_) => {}
                    }
                });
            }
        });
    }
    if let Some(status) = &state.status {
        ui.small(status);
    }
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs(1));
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
}

fn expiration_label(expires_at: i64, now: i64) -> String {
    let seconds = expires_at.saturating_sub(now).max(0) as u64;
    let remaining = if seconds >= 3_600 {
        format!("{} h {} min", seconds / 3_600, (seconds % 3_600) / 60)
    } else if seconds >= 60 {
        format!("{} min {} s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds} s")
    };
    format!("endet in {remaining} (Endzeit {expires_at} Unix)")
}

fn offer_expiration_label(expires_at: i64, now: i64) -> String {
    if expires_at <= now {
        "Zeit abgelaufen; Worker-Bestaetigung steht aus".into()
    } else {
        expiration_label(expires_at, now)
    }
}

fn offer_phase_label(phase: DiscoveryOfferPhase) -> &'static str {
    match phase {
        DiscoveryOfferPhase::Prepared => "vorbereitet, Server-Bestaetigung steht aus",
        DiscoveryOfferPhase::Published => "vom Server bestaetigt",
    }
}

fn pin_guidance(
    ui: &mut egui::Ui,
    pin: &super::share_discovery_state::DiscoveryPinDraft,
) {
    if pin.byte_len() > crate::share::DISCOVERY_PIN_MAX_BYTES {
        ui.colored_label(
            egui::Color32::from_rgb(220, 100, 90),
            format!(
                "PIN ist {} Bytes lang; maximal {} Bytes sind erlaubt. Es wird nichts gekuerzt.",
                pin.byte_len(),
                crate::share::DISCOVERY_PIN_MAX_BYTES
            ),
        );
    } else if pin.trivially_guessable() {
        ui.colored_label(
            egui::Color32::from_rgb(225, 155, 70),
            "Leere PINs und \"0\" sind erlaubt, aber trivial zu erraten.",
        );
    }
    ui.small(
        "Die PIN wird als exakte UTF-8-Bytefolge verwendet und nicht dauerhaft gespeichert.",
    );
}
