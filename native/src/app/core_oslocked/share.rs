use super::prelude::*;
use super::*;

impl App {
    /// Start the share service if a rendezvous server is configured. Returns
    /// whether a service is available.
    pub(in crate::app) fn ensure_share(&mut self) -> bool {
        if self.share.is_some() {
            return true;
        }
        let server = self.share_server.trim().to_string();
        if server.is_empty() {
            self.share_status = "Kein Server eingetragen (Einstellungen → TEILEN)".to_string();
            return false;
        }
        let device = if self.share_device_draft.trim().is_empty() {
            default_device_name()
        } else {
            self.share_device_draft.trim().to_string()
        };
        match crate::share::ShareService::start(server, device) {
            Ok(svc) => {
                self.share = Some(svc);
                true
            }
            Err(e) => {
                self.error_msg = Some(format!("Teilen-Dienst: {}", e));
                false
            }
        }
    }

    pub(in crate::app) fn share_cmd(&mut self, c: crate::share::ShareCmd) {
        if self.ensure_share() {
            if let Some(svc) = &self.share {
                svc.cmd(c);
            }
        }
    }

    /// Lazily start Quick Share LAN discovery while the Teilen view is open, and
    /// drain discovered devices.
    pub(in crate::app) fn drain_quickshare(&mut self) {
        if self.show_share && self.quickshare.is_none() {
            let name = if self.share_device_draft.trim().is_empty() {
                default_device_name()
            } else {
                self.share_device_draft.trim().to_string()
            };
            self.quickshare = crate::quickshare::QuickShare::start(&name);
        }
        if let Some(qs) = &self.quickshare {
            for list in qs.events.try_iter() {
                self.qs_devices = list;
            }
        }
    }

    pub(in crate::app) fn drain_share(&mut self) {
        let events: Vec<crate::share::ShareEvent> = match &self.share {
            Some(svc) => svc.events.try_iter().collect(),
            None => return,
        };
        for ev in events {
            use crate::share::ShareEvent as E;
            match ev {
                E::Status(s) => self.share_status = s,
                E::Error(e) => {
                    self.share_status = format!("Fehler: {}", e);
                    self.error_msg = Some(format!("Teilen: {}", e));
                }
                E::Roster(r) => self.share_roster = r,
                E::Incoming { id, from, files } => {
                    self.share_incoming.push((id, from, files));
                    self.show_share = true;
                }
                E::Progress { done, total } => self.share_progress = Some((done, total)),
                E::Received { count, dir } => {
                    self.share_progress = None;
                    self.share_status = format!("✓ {} empfangen → {}", count, dir);
                    self.notice = Some((
                        format!("📥 {} Datei(en) empfangen", count),
                        std::time::Instant::now(),
                    ));
                }
                E::Sent { count } => {
                    self.share_progress = None;
                    self.share_status = format!("✓ {} gesendet", count);
                }
            }
        }
    }

    /// Local file paths in the current selection (sharing sends local files;
    /// remote selections aren't supported yet).
    pub(in crate::app) fn selected_local_files(&self) -> Vec<String> {
        if self.remote.is_some() {
            return Vec::new();
        }
        self.entries
            .iter()
            .filter(|e| !e.is_dir && self.selection.contains(&e.key()))
            .map(|e| e.path.replace('/', std::path::MAIN_SEPARATOR_STR))
            .collect()
    }

    pub(in crate::app) fn ui_share(&mut self, ctx: &egui::Context) {
        let mut open = self.show_share;
        let mut pair_show = false;
        let mut pair_connect = false;
        let mut room_join = false;
        let mut leave = false;
        let mut send = false;
        let mut answer: Option<(u64, bool)> = None;

        let roster = self.share_roster.clone();
        let incoming = self.share_incoming.clone();
        let status = self.share_status.clone();
        let progress = self.share_progress;
        let my_code = self.share_my_code.clone();
        let fingerprint = self.share.as_ref().map(|s| s.fingerprint.clone()).unwrap_or_default();
        let sel = self.selected_local_files().len();
        let qs_devices = self.qs_devices.clone();

        egui::Window::new("📡 Teilen — Geräte & Räume")
            .open(&mut open)
            .resizable(true)
            .default_size([460.0, 520.0])
            .show(ctx, |ui| {
                if self.share_server.trim().is_empty() {
                    ui.colored_label(
                        Color32::from_rgb(255, 185, 120),
                        "Kein Rendezvous-Server eingetragen.",
                    );
                    ui.label("Einstellungen → TEILEN: Server-Adresse (host:port) setzen.");
                    return;
                }
                if !fingerprint.is_empty() {
                    ui.label(
                        RichText::new(format!("Dieses Gerät: {}", fingerprint))
                            .small()
                            .color(Color32::from_gray(140)),
                    );
                }

                ui.add_space(6.0);
                ui.label(RichText::new("DIREKT KOPPELN").small().color(Color32::from_gray(140)));
                ui.horizontal(|ui| {
                    if ui.button("Code anzeigen").on_hover_text("Erzeugt einen Code; das andere Gerät gibt ihn ein").clicked() {
                        pair_show = true;
                    }
                    if !my_code.is_empty() {
                        ui.label(RichText::new(&my_code).monospace().strong().size(18.0));
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.share_code_input).hint_text("Code vom anderen Gerät").desired_width(160.0));
                    if ui.button("Verbinden").clicked() {
                        pair_connect = true;
                    }
                });

                ui.add_space(8.0);
                ui.label(RichText::new("RAUM").small().color(Color32::from_gray(140)));
                ui.horizontal(|ui| {
                    if ui.button("Raum erstellen").clicked() {
                        room_join = true; // generates a code below
                    }
                    if ui.button("Beitreten").clicked() {
                        room_join = true;
                    }
                });

                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Verbundene Geräte ({})", roster.len())).strong());
                    if !roster.is_empty() && ui.small_button("Verlassen").clicked() {
                        leave = true;
                    }
                });
                if roster.is_empty() {
                    ui.colored_label(Color32::from_gray(140), "(noch keine — Code teilen oder Raum beitreten)");
                }
                for d in &roster {
                    ui.label(format!("● {}  ({})", d.device, d.fingerprint));
                }

                ui.add_space(6.0);
                if ui
                    .add_enabled(sel > 0 && !roster.is_empty(), egui::Button::new(format!("⮝ {} ausgewählte Datei(en) senden", sel)))
                    .on_hover_text("Sendet die in der Liste markierten lokalen Dateien an alle verbundenen Geräte")
                    .clicked()
                {
                    send = true;
                }
                if sel == 0 {
                    ui.label(RichText::new("Markiere lokale Dateien in der Liste, um sie zu senden.").small().color(Color32::from_gray(120)));
                }

                if let Some((done, total)) = progress {
                    let frac = if total > 0 { done as f32 / total as f32 } else { 0.0 };
                    ui.add(egui::ProgressBar::new(frac).show_percentage());
                }
                if !status.is_empty() {
                    ui.label(RichText::new(&status).small().color(Color32::from_gray(150)));
                }

                // Quick Share (Android) devices seen on the LAN.
                ui.separator();
                egui::CollapsingHeader::new(format!("📱 Quick Share (LAN) — {} gefunden", qs_devices.len()))
                    .id_salt("qs_devices")
                    .show(ui, |ui| {
                        if qs_devices.is_empty() {
                            ui.colored_label(Color32::from_gray(140), "(Suche… Android: Quick Share auf „Alle“ stellen)");
                        }
                        for d in &qs_devices {
                            ui.label(format!("📱 {}  {}", d.name, d.addr));
                        }
                        ui.label(
                            RichText::new(
                                "Übertragung zu/von Quick Share ist in Arbeit (UKEY2/Protobuf, \
                                 siehe docs/QUICKSHARE.md). Für Geräte mit Smart Explorer nutze \
                                 oben Direkt koppeln / Raum.",
                            )
                            .small()
                            .color(Color32::from_gray(120)),
                        );
                    });

                if !incoming.is_empty() {
                    ui.separator();
                    ui.label(RichText::new("EINGEHEND").small().color(Color32::from_gray(140)));
                    for (id, from, files) in &incoming {
                        let total: u64 = files.iter().map(|(_, s)| *s).sum();
                        ui.label(format!("{} möchte {} Datei(en) senden ({})", from, files.len(), format_bytes(total)));
                        ui.horizontal(|ui| {
                            if ui.button("Annehmen").clicked() {
                                answer = Some((*id, true));
                            }
                            if ui.button("Ablehnen").clicked() {
                                answer = Some((*id, false));
                            }
                        });
                    }
                }
            });
        self.show_share = open;

        if pair_show {
            let code = crate::share::gen_code();
            self.share_my_code = code.clone();
            self.share_room = false;
            self.share_cmd(crate::share::ShareCmd::Pair(code));
        }
        if pair_connect {
            let code = self.share_code_input.trim().to_string();
            if !code.is_empty() {
                self.share_my_code.clear();
                self.share_cmd(crate::share::ShareCmd::Pair(code));
            }
        }
        if room_join {
            let code = if self.share_code_input.trim().is_empty() {
                let c = crate::share::gen_code();
                self.share_my_code = c.clone();
                c
            } else {
                self.share_code_input.trim().to_string()
            };
            self.share_room = true;
            self.share_cmd(crate::share::ShareCmd::JoinRoom(code));
        }
        if leave {
            self.share_roster.clear();
            self.share_my_code.clear();
            self.share_cmd(crate::share::ShareCmd::Leave);
        }
        if send {
            let files = self.selected_local_files();
            if files.is_empty() {
                self.error_msg = Some("Keine lokalen Dateien ausgewählt.".to_string());
            } else {
                self.share_cmd(crate::share::ShareCmd::Send(files));
            }
        }
        if let Some((id, accept)) = answer {
            self.share_incoming.retain(|(i, _, _)| *i != id);
            self.share_cmd(crate::share::ShareCmd::Answer { id, accept });
        }
    }

}
