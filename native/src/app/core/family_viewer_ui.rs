//! Local family menu, metadata and resize presentation on the existing viewer.
use super::super as app;
use super::*;
use crate::screen::core::wire_control::{ControlAction, ControlRequest, ViewerFrame};

impl FamilyViewerApp {
    pub(super) fn can_control(&self) -> bool {
        self.session.as_ref().is_some_and(|session| {
            let info = session.info();
            session.mode == crate::screen::core::wire_hello::SessionMode::Control
                && info.hello.as_ref().is_some_and(|h| {
                    h.authorization.control != crate::share::access_grant::CapabilityDecision::Off
                })
                && info.blocked.is_none()
                && !info.control_denied
                && !info.is_ended()
        })
    }

    pub(super) fn metadata(&mut self, ctx: &egui::Context) {
        let snapshot = self.snapshot();
        let root = snapshot
            .as_ref()
            .and_then(|s| s.members.iter().find(|m| m.identity == s.root));
        let app = root
            .map(|m| m.app.as_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("Fernfenster");
        let title = format!("{app} – {}", self.device);
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        if let Some(session) = &self.session {
            let info = session.info();
            if let Some(icon) = info.family_icon.clone() {
                if self
                    .icon
                    .as_ref()
                    .is_none_or(|old| !Arc::ptr_eq(old, &icon))
                {
                    ctx.send_viewport_cmd(ViewportCommand::Icon(Some(icon.clone())));
                    self.icon = Some(icon);
                }
            } else if self.icon.take().is_some() {
                ctx.send_viewport_cmd(ViewportCommand::Icon(None));
            }
            if let Some(icon) = root.and_then(|m| m.icon.as_ref()) {
                let mut icons = info
                    .icons
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if !icons.contains(&info.icon_peer, icon) {
                    icons
                        .requests
                        .request(crate::screen::core::app_icon::IconRequest {
                            target: crate::screen::core::app_icon::IconTarget::App {
                                app_id: icon.app_id.clone(),
                            },
                            icon: icon.clone(),
                        });
                }
            }
        }
    }

    pub(super) fn menu(&mut self, ctx: &egui::Context) {
        let was_open = self.menu;
        self.local_block = was_open;
        egui::Area::new(egui::Id::new("family-menu-button"))
            .anchor(egui::Align2::LEFT_TOP, [8.0, 8.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let response = ui.button("Menü").on_hover_text("Fenstermenü · Strg+Alt+M");
                self.local_block |= response.hovered() || response.contains_pointer();
                if response.clicked() {
                    self.menu = !self.menu;
                }
            });
        if was_open != self.menu {
            if let Some(session) = &self.session {
                self.widget.release_input(session);
                app::family_viewer_session::disarm(session);
            }
        }
        if !self.menu {
            return;
        }
        let mut open = self.menu;
        egui::Window::new("Fenstermenü · Strg+Alt+M")
            .id(egui::Id::new("family-menu"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(&self.title);
                if let Some(session) = &self.session {
                    let info = session.info();
                    ui.label(format!(
                        "Verbindung: {} · {:?}",
                        info.path.as_deref().unwrap_or("wird hergestellt"),
                        info.state
                    ));
                    if let Some(family) = &info.family {
                        if let Some(notice) = &family.notice {
                            ui.label(notice);
                        }
                    }
                    if let Some(reason) = info.reason.as_ref().or(info.notice.as_ref()) {
                        ui.label(reason);
                    }
                }
                let allowed = self.control && self.can_control();
                if ui
                    .add_enabled(
                        self.target.control,
                        egui::Checkbox::new(&mut self.control, "Steuern"),
                    )
                    .on_hover_text("Steuern benötigt eine mit Steuerrecht geöffnete Sitzung.")
                    .changed()
                {
                    if let Some(session) = &self.session {
                        self.widget.release_input(session);
                        app::family_viewer_session::disarm(session);
                        session.send(ViewerFrame::ControlRequest(ControlRequest {
                            action: if self.control {
                                ControlAction::Take
                            } else {
                                ControlAction::Release
                            },
                        }));
                    }
                }
                if !self.target.control {
                    ui.label("Zum Steuern in der Fensterliste „Steuern“ wählen und neu öffnen.");
                }
                if ui
                    .add_enabled(allowed, egui::Button::new("Fensterfokus bestätigen"))
                    .clicked()
                {
                    if let (Some(session), Some(snapshot)) = (&self.session, self.snapshot()) {
                        let identity = snapshot
                            .focused_target()
                            .map(|t| t.identity)
                            .unwrap_or(snapshot.root);
                        app::family_viewer_session::operate(
                            session,
                            identity,
                            WindowOperation::Focus,
                        );
                    }
                }
                ui.checkbox(&mut self.remote_resize, "Lokale Größe an Programm senden");
                let close = allowed
                    && self.snapshot().is_some_and(|s| {
                        s.members
                            .iter()
                            .any(|m| m.identity == s.root && m.operations.close)
                    });
                if ui
                    .add_enabled(close, egui::Button::new("Programm schließen"))
                    .clicked()
                {
                    self.close_program();
                }
                if !close {
                    ui.label(
                        "Reguläres Schließen ist für das Hauptfenster aktuell nicht freigegeben.",
                    );
                }
                if ui.button("Fensterzustand aktualisieren").clicked() {
                    if let Some(session) = &self.session {
                        session.send(ViewerFrame::FamilySnapshot);
                    }
                }
                ui.checkbox(&mut self.diagnostics, "Diagnose anzeigen");
                ui.separator();
                if ui.button("Verbindung trennen").clicked() {
                    self.stop(ctx);
                }
            });
        self.menu = open;
    }

    pub(super) fn resize(&mut self, display: FrameSize) {
        if self.last_display.is_none() {
            self.last_display = Some(display);
            self.resize_sent = Some(display);
            return;
        }
        if self.last_display != Some(display) {
            self.last_display = Some(display);
            self.resize_at = Some(Instant::now());
            return;
        }
        if !self.remote_resize
            || !self.control
            || !self.can_control()
            || self
                .resize_at
                .is_none_or(|at| at.elapsed() < Duration::from_millis(400))
            || self.resize_sent == Some(display)
        {
            return;
        }
        let Some(snapshot) = self.snapshot() else {
            return;
        };
        let Some(root) = snapshot
            .members
            .iter()
            .find(|m| m.identity == snapshot.root && m.operations.resize)
        else {
            return;
        };
        let visible: Vec<_> = snapshot
            .members
            .iter()
            .filter(|m| m.visible && !m.minimized)
            .collect();
        let width = visible
            .iter()
            .map(|m| i64::from(m.geometry.x) + i64::from(m.geometry.width))
            .max()
            .unwrap_or(0)
            - visible
                .iter()
                .map(|m| i64::from(m.geometry.x))
                .min()
                .unwrap_or(0);
        let height = visible
            .iter()
            .map(|m| i64::from(m.geometry.y) + i64::from(m.geometry.height))
            .max()
            .unwrap_or(0)
            - visible
                .iter()
                .map(|m| i64::from(m.geometry.y))
                .min()
                .unwrap_or(0);
        let wanted = FrameSize::new(
            (i64::from(display.width) - (width - i64::from(root.geometry.width))).max(1) as u32,
            (i64::from(display.height) - (height - i64::from(root.geometry.height))).max(1) as u32,
        );
        if let Some(session) = &self.session {
            app::family_viewer_session::operate(
                session,
                root.identity.clone(),
                WindowOperation::Resize {
                    width: wanted.width,
                    height: wanted.height,
                },
            );
        }
        self.resize_sent = Some(display);
    }
}
