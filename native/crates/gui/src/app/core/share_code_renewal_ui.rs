//! Direkt-Code renewal on the Share pages (E17, plan M22): the transition
//! state and the devices it waits for. The values are the `CodeRenewalView`
//! of the worker snapshot, the one `se share status`, the MCP status and the
//! Android app read as well.
use super::*;
use crate::daemon::CodeRenewalView;

/// What the Share pages know about the renewal.
#[derive(Default)]
pub(in crate::app) struct CodeRenewalUi {
    /// „Wirklich neu generieren“ is asked.
    pub(in crate::app) confirm: bool,
    view: Option<CodeRenewalView>,
}

impl CodeRenewalUi {
    /// Keeps the newest view; `true` when its generation changed.
    fn replace(&mut self, view: Option<CodeRenewalView>) -> bool {
        let generation = |view: &Option<CodeRenewalView>| view.as_ref().map(|view| view.generation);
        let changed = generation(&self.view) != generation(&view);
        self.view = view;
        changed
    }

    /// The summary line on top of every Share page, with the notes about a
    /// waiting renewal and a failed bookkeeping.
    pub(in crate::app) fn banner(&self, ui: &mut egui::Ui) {
        let Some(view) = &self.view else {
            return;
        };
        if view.active {
            ui.add(egui::Label::new(RichText::new(summary(view)).color(theme::warning(ui))).wrap());
        }
        if view.deferred {
            ui.add(egui::Label::new(DEFERRED).wrap());
        }
        if let Some(error) = &view.error {
            ui.add(
                egui::Label::new(
                    RichText::new(format!("Fehler bei der Code-Erneuerung: {error}"))
                        .color(theme::danger(ui)),
                )
                .wrap(),
            );
        }
    }

    /// The pending devices below the Direkt-Code (page „Dieses Gerät“).
    pub(in crate::app) fn devices(&self, ui: &mut egui::Ui) {
        if let Some(line) = self.view.as_ref().and_then(names_line) {
            ui.add(egui::Label::new(line).wrap());
        }
    }
}

const DEFERRED: &str = "Eine weitere Erneuerung wartet, bis ein laufender Übergang endet.";

/// „Code wird erneuert, 2 Geräte stehen aus (davon 1 mit älterer Version)“.
fn summary(view: &CodeRenewalView) -> String {
    let pending = view.pending_devices;
    let mut text = if pending == 1 {
        "Code wird erneuert, 1 Gerät steht aus".to_string()
    } else {
        format!("Code wird erneuert, {pending} Geräte stehen aus")
    };
    match view.legacy_devices {
        0 => {}
        1 => text.push_str(" (davon 1 mit älterer Version)"),
        legacy => text.push_str(&format!(" (davon {legacy} mit älteren Versionen)")),
    }
    text
}

/// „Ausstehend: A, B und 3 weitere“; the view names at most 32 devices.
fn names_line(view: &CodeRenewalView) -> Option<String> {
    if !view.active || view.pending_names.is_empty() {
        return None;
    }
    let mut line = format!("Ausstehend: {}", view.pending_names.join(", "));
    let more = view
        .pending_devices
        .saturating_sub(view.pending_names.len());
    if more > 0 {
        line.push_str(&format!(" und {more} weitere"));
    }
    Some(line)
}

impl App {
    /// Folds in the view of a successful snapshot. When the renewal
    /// generation changed, the daemon renewed the Direkt-Code: the cached
    /// identity is read again, else the page keeps showing the old code
    /// (new requests with it would be refused).
    pub(in crate::app) fn apply_code_renewal(&mut self, view: Option<CodeRenewalView>) {
        if !self.share_code_renewal.replace(view) {
            return;
        }
        let Some(name) = self
            .share_identity
            .as_ref()
            .map(|identity| identity.device_name.clone())
        else {
            return;
        };
        match crate::share::ShareIdentity::load_or_create(name) {
            Ok(identity) => self.share_identity = Some(identity),
            Err(error) => {
                self.append_share_diag(format!("Direkt-Code nach der Erneuerung laden: {error}"))
            }
        }
    }
}

#[cfg(test)]
#[path = "share_code_renewal_task_tests.rs"]
mod task_tests;
