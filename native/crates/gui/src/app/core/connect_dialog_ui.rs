//! The dialog "Neue Verbindung" / "Verbindung bearbeiten": every field of a
//! saved remote (protocol, host, port, user, start path or UNC path, password,
//! key file and passphrase, SSH agent, remote agent, label). SMB is offered on
//! every desktop platform (the shared connector speaks SMB 2/3 itself); the
//! Windows network share (UNC with WNet login) only where Windows provides it.
//! Editing saves through `connect::save_edited_connection`, the path of the
//! Android form and `se connections edit`.
use super::prelude::*;
use super::*;
use crate::app::theme;
use crate::creds::Protocol;

/// Protocols the dialog offers on this platform, in display order.
fn offered_protocols() -> Vec<Protocol> {
    let mut protocols = vec![
        Protocol::Sftp,
        Protocol::Ftp,
        Protocol::Ftps,
        Protocol::Webdav,
        Protocol::Smb,
    ];
    if unc_connections_available() {
        protocols.push(Protocol::Share);
    }
    protocols
}

fn protocol_label(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Sftp => "SFTP",
        Protocol::Ftp => "FTP",
        Protocol::Ftps => "FTPS",
        Protocol::Webdav => "WebDAV (HTTPS)",
        Protocol::Smb => "SMB (Windows-/Samba-Freigabe)",
        Protocol::Share => "Netzlaufwerk (UNC)",
    }
}

impl App {
    pub(in crate::app) fn ui_connect_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_connect {
            return;
        }
        let editing = self.connect_edit.clone();
        let mut do_connect = false;
        let mut do_save = false;
        let mut close = false;
        let mut open = true;
        let title = if editing.is_some() {
            "Verbindung bearbeiten"
        } else {
            "Neue Verbindung"
        };
        super::dialog_geometry::window(ctx, "connect_dialog_ui.0", title)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([520.0, 480.0])
            .max_size(theme::window_content_limit(ctx))
            .constrain_to(ctx.screen_rect().shrink(16.0))
            .vscroll(true)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                let f = &mut self.connect_form;
                if let Some(previous) = &editing {
                    ui.label(
                        RichText::new(format!("Gespeichert als {}", previous.account()))
                            .small()
                            .color(theme::muted(ui)),
                    );
                }
                egui::ComboBox::from_label("Protokoll")
                    .selected_text(protocol_label(f.protocol))
                    .show_ui(ui, |ui| {
                        for p in offered_protocols() {
                            if ui.selectable_label(f.protocol == p, protocol_label(p)).clicked() {
                                // An untouched standard port follows the protocol.
                                let old_default = f.protocol.default_port().to_string();
                                f.protocol = p;
                                if p != Protocol::Share
                                    && (f.port.trim().is_empty() || f.port.trim() == old_default)
                                {
                                    f.port = p.default_port().to_string();
                                }
                            }
                        }
                    });
                ui.add_space(4.0);

                egui::Grid::new("connect_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        if f.protocol == Protocol::Share {
                            ui.label("Freigabe (UNC)");
                            ui.add(
                                egui::TextEdit::singleline(&mut f.unc)
                                    .hint_text(r"\\server\share")
                                    .desired_width(f32::INFINITY),
                            );
                            ui.end_row();
                        } else {
                            ui.label("Host");
                            ui.add(
                                egui::TextEdit::singleline(&mut f.host)
                                    .hint_text("host.example.com")
                                    .desired_width(f32::INFINITY),
                            );
                            ui.end_row();
                            ui.label("Port");
                            ui.add(egui::TextEdit::singleline(&mut f.port).desired_width(70.0));
                            ui.end_row();
                        }
                        ui.label("Benutzer");
                        let user_hint = if matches!(f.protocol, Protocol::Smb | Protocol::Share) {
                            r"benutzer oder DOMÄNE\benutzer"
                        } else {
                            ""
                        };
                        ui.add(
                            egui::TextEdit::singleline(&mut f.user)
                                .hint_text(user_hint)
                                .desired_width(f32::INFINITY),
                        );
                        ui.end_row();
                        if f.protocol == Protocol::Share {
                            ui.label("Passwort");
                            ui.add(
                                egui::TextEdit::singleline(&mut f.password)
                                    .password(true)
                                    .desired_width(f32::INFINITY),
                            );
                            ui.end_row();
                        } else {
                            ui.label("Startpfad");
                            let root_hint = if f.protocol == Protocol::Smb {
                                "/freigabe/ordner"
                            } else {
                                "/"
                            };
                            ui.add(
                                egui::TextEdit::singleline(&mut f.root)
                                    .hint_text(root_hint)
                                    .desired_width(f32::INFINITY),
                            );
                            ui.end_row();
                        }
                    });
                if f.protocol == Protocol::Smb && !crate::smb::root_has_share(&f.root) {
                    ui.colored_label(
                        theme::warning(ui),
                        "Bei SMB beginnt der Startpfad mit der Freigabe, z. B. /dokumente/projekte.",
                    );
                }

                if f.protocol == Protocol::Sftp {
                    if ui
                        .checkbox(&mut f.use_key, "Mit Schlüsseldatei anmelden")
                        .changed()
                        && f.use_key
                    {
                        f.use_ssh_agent = false;
                    }
                    if ui
                        .checkbox(&mut f.use_ssh_agent, "Mit SSH-Agent anmelden")
                        .on_hover_text(
                            "Meldet sich mit den Schlüsseln des laufenden SSH-Agents an \
                             (ssh-agent, Windows-OpenSSH-Agent oder Pageant, auch \
                             FIDO2-Schlüssel); es wird kein Geheimnis gespeichert.",
                        )
                        .changed()
                        && f.use_ssh_agent
                    {
                        f.use_key = false;
                    }
                    ui.checkbox(&mut f.use_agent, "Remote-Agent verwenden")
                        .on_hover_text(
                            "Lädt beim Verbinden einen kleinen Helfer auf den Server und führt \
                             Erkundung/Analyse dort lokal aus (statt vieler Netzwerk-Roundtrips). \
                             Opt-in; fällt bei Problemen automatisch auf normales SFTP zurück. \
                             Der Zugriff bleibt bei Problemen über SFTP möglich.",
                        );
                }
                let keep_hint = if editing.is_some() {
                    "leer = gespeichertes behalten"
                } else {
                    ""
                };
                if f.protocol == Protocol::Sftp && f.use_key {
                    ui.horizontal(|ui| {
                        ui.label("Schlüssel");
                        ui.add(egui::TextEdit::singleline(&mut f.keyfile).desired_width(220.0));
                        if ui.button("…").clicked() {
                            if let Some(p) = rfd::FileDialog::new().pick_file() {
                                f.keyfile = p.to_string_lossy().replace('\\', "/");
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Passphrase");
                        ui.add(
                            egui::TextEdit::singleline(&mut f.passphrase)
                                .password(true)
                                .hint_text(keep_hint)
                                .desired_width(220.0),
                        );
                    });
                } else if f.protocol != Protocol::Share
                    && !(f.protocol == Protocol::Sftp && f.use_ssh_agent)
                {
                    ui.horizontal(|ui| {
                        ui.label("Passwort");
                        ui.add(
                            egui::TextEdit::singleline(&mut f.password)
                                .password(true)
                                .hint_text(keep_hint)
                                .desired_width(f32::INFINITY),
                        );
                    });
                }

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if editing.is_none() {
                        ui.checkbox(&mut f.save, "Speichern");
                    } else {
                        ui.label("Bezeichnung");
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut f.label)
                            .hint_text("Bezeichnung (optional)")
                            .desired_width(f32::INFINITY),
                    );
                });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if editing.is_some() {
                        if ui.button(RichText::new("Speichern").strong()).clicked() {
                            do_save = true;
                        }
                        if ui.button("Abbrechen").clicked() {
                            close = true;
                        }
                        ui.label(
                            RichText::new("Gilt ab der nächsten Verbindung.")
                                .small()
                                .color(theme::muted(ui)),
                        );
                    } else if self.connecting {
                        ui.spinner();
                        ui.label("Verbinde…");
                        if ui.button("Abbrechen").clicked() {
                            close = true;
                        }
                    } else {
                        if ui.button(RichText::new("Verbinden").strong()).clicked() {
                            do_connect = true;
                        }
                        if ui.button("Abbrechen").clicked() {
                            close = true;
                        }
                    }
                });
            });
        if !open {
            close = true;
        }
        if do_save {
            self.save_connection_edit();
        } else if do_connect {
            let form = self.connect_form.clone();
            self.begin_connect(form, None);
        } else if close {
            self.connect_rx = None;
            self.connecting = false;
            self.show_connect = false;
            self.connect_edit = None;
        }
    }
}
