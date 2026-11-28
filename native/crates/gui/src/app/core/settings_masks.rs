//! Settings page „Maskenlisten“ (docs/oberflaechen/masken.md): mask lists for
//! AI agents with their entries (value hidden, shown on click only here),
//! entry form (kind, label, tolerance level or numbers, Groß/Klein, ganzes
//! Wort), protected folders (local folder dialog or any se target, remote
//! included), lock, enabled state, allowed foreign MCP servers and the state
//! of the hardening in Claude Code and Codex. A change that weakens the
//! protection asks through the system dialog (UAC/polkit) first; the work
//! runs on a worker thread (`settings_masks_work.rs`).
#[path = "settings_masks_form.rs"]
mod form;
#[path = "settings_masks_work.rs"]
mod work;

use super::super::prelude::*;
use crate::app::App;
use crate::masks::{EntryId, EntrySummary, ListId, ListSummary};
use form::EntryForm;
#[cfg(test)]
pub(in crate::app) use form::{describe, level_text, EntryForm as TestEntryForm, Level};
use serde_json::Value;
use std::collections::HashMap;
#[cfg(test)]
pub(in crate::app) use work::summary as work_summary;
pub(in crate::app) use work::Op;

#[derive(Default)]
pub(in crate::app) struct MasksPage {
    lists: Option<Result<Vec<ListSummary>, String>>,
    loading: Option<Receiver<Result<Vec<ListSummary>, String>>>,
    operation: Option<Receiver<work::Done>>,
    status: Option<Result<Value, String>>,
    status_rx: Option<Receiver<Result<Value, String>>>,
    message: Option<Result<String, String>>,
    new_list: String,
    add_form: EntryForm,
    editing: Option<(EntryId, EntryForm)>,
    revealed: HashMap<EntryId, String>,
    scope_text: String,
    server_text: String,
    /// A click on "Ordner wählen …" for this list: the page opens the in-app
    /// folder picker (local drives and saved remotes) after drawing.
    pick_scope: Option<(ListId, String)>,
}

impl MasksPage {
    fn reload(&mut self) {
        self.loading = work::spawn("mask-lists", work::load);
        if self.loading.is_none() {
            self.lists = Some(Err("Die Maskenlisten konnten nicht gelesen werden.".into()));
        }
        self.status_rx = work::spawn("mask-status", work::status);
    }

    fn poll(&mut self) {
        if let Some(result) = self.loading.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.lists = Some(result);
            self.loading = None;
        }
        if let Some(result) = self.status_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.status = Some(result);
            self.status_rx = None;
        }
        if let Some(done) = self.operation.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.operation = None;
            if let Some((entry, value)) = done.revealed {
                self.revealed.insert(entry, value);
            }
            if done.message.is_some() {
                self.message = done.message;
            }
            if done.reload {
                self.reload();
            }
        }
    }

    fn busy(&self) -> bool {
        self.loading.is_some() || self.operation.is_some() || self.status_rx.is_some()
    }

    fn start(&mut self, op: Op, list: &str) {
        if self.operation.is_some() {
            return;
        }
        let list = list.to_string();
        self.operation = work::spawn("mask-change", move || work::run(op, list));
        if self.operation.is_none() {
            self.message = Some(Err("Die Änderung konnte nicht gestartet werden.".into()));
        }
    }

    /// Takes a picked folder (local path or se target) as a protected path
    /// of `list`; the in-app folder picker hands its choice in here.
    pub(in crate::app) fn add_scope(&mut self, list: ListId, name: &str, target: String) {
        if !target.trim().is_empty() {
            self.start(Op::AddScope(list, target), name);
        }
    }
}

impl App {
    pub(in crate::app) fn ui_settings_masks(&mut self, ui: &mut egui::Ui) {
        let page = &mut self.settings.masks;
        page.poll();
        if page.lists.is_none() && page.loading.is_none() {
            page.reload();
        }
        if page.busy() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        ui.label("Maskenlisten ersetzen Werte durch Platzhalter (⟦Bezeichnung-1⟧), bevor KI-Agenten sie über Smart Explorer sehen, und führen die Platzhalter in ihren Eingaben zurück. Schützt eine aktive Liste einen Ordner, arbeiten Agenten in Claude Code und Codex nur noch über die Smart-Explorer-Werkzeuge.");
        status_section(ui, page);
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.label("Neue Liste:");
            ui.add(
                egui::TextEdit::singleline(&mut page.new_list)
                    .hint_text("Name")
                    .desired_width(200.0),
            );
            let enabled = !page.new_list.trim().is_empty() && page.operation.is_none();
            if ui
                .add_enabled(enabled, egui::Button::new("Anlegen"))
                .clicked()
            {
                let name = page.new_list.trim().to_string();
                page.new_list.clear();
                page.start(Op::Create(name.clone()), &name);
            }
            if ui
                .add_enabled(!page.busy(), egui::Button::new("Neu laden"))
                .clicked()
            {
                page.reload();
            }
            if page.busy() {
                ui.spinner();
            }
        });
        match &page.message {
            Some(Ok(text)) => {
                ui.colored_label(super::super::theme::success(ui), text);
            }
            Some(Err(text)) => {
                ui.colored_label(super::super::theme::danger(ui), text);
            }
            None => {}
        }
        ui.separator();
        let lists = match &page.lists {
            None => {
                ui.label("Maskenlisten werden gelesen …");
                return;
            }
            Some(Err(error)) => {
                ui.colored_label(super::super::theme::danger(ui), error);
                return;
            }
            Some(Ok(lists)) => lists.clone(),
        };
        if lists.is_empty() {
            ui.label("Es gibt noch keine Maskenliste.");
        }
        for list in &lists {
            egui::CollapsingHeader::new(format!(
                "{} · {} · {} · {} Einträge · {} Schutzpfade",
                list.name,
                if list.enabled { "aktiv" } else { "inaktiv" },
                if list.locked { "gesperrt" } else { "offen" },
                list.entries.len(),
                list.scopes.len()
            ))
            .id_salt(("mask_list", list.id.0))
            .show(ui, |ui| list_body(ui, page, list));
        }
        if let Some((list, name)) = self.settings.masks.pick_scope.take() {
            self.open_picker(
                crate::app::picker_types::PickerPurpose::MaskScope { list, name },
                "",
            );
        }
    }
}

fn status_section(ui: &mut egui::Ui, page: &mut MasksPage) {
    let muted = super::super::theme::muted(ui);
    let Some(status) = &page.status else {
        ui.label(RichText::new("Zustand wird gelesen …").color(muted));
        return;
    };
    let status = match status {
        Ok(status) => status.clone(),
        Err(error) => {
            ui.colored_label(
                super::super::theme::warning(ui),
                format!("Zustand: {error}"),
            );
            return;
        }
    };
    let lockdown = match status["lockdown"].as_str() {
        Some("on") => "Agenten-Sperre aktiv",
        Some("off") => "Agenten-Sperre aus (keine aktive Liste schützt einen Ordner)",
        _ => "Agenten-Sperre: Zustand nicht lesbar, Agenten bleiben gesperrt",
    };
    ui.label(lockdown);
    for (host, label) in [("claude", "Claude Code"), ("codex", "Codex")] {
        let state = &status["hardening"][host];
        if state.is_null() {
            continue;
        }
        let text = match state["state"].as_str() {
            Some("current") => "verankert",
            Some("outdated") => "veraltet",
            Some("absent") => "nicht verankert",
            Some("unreadable") => "nicht lesbar",
            Some("conflict") => "Datei ungültig",
            _ => "unbekannt",
        };
        ui.label(
            RichText::new(format!(
                "{label}: verwaltete Einstellungen {text} ({})",
                state["path"].as_str().unwrap_or("?")
            ))
            .color(muted),
        );
    }
    if let Some(error) = status["hardening"]["error"].as_str() {
        ui.colored_label(
            super::super::theme::warning(ui),
            format!("Härtung: {error}"),
        );
    }
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(page.operation.is_none(), egui::Button::new("Härtung jetzt verankern"))
            .on_hover_text("Schreibt Hook und Leseregeln in die verwalteten Einstellungen von Claude Code und Codex (Systemdialog).")
            .clicked()
        {
            page.start(Op::Harden, "");
        }
    });
    let servers: Vec<String> = status["allowed_mcp_servers"]
        .as_array()
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    super::super::theme::section(ui, "Erlaubte fremde MCP-Server während der Sperre");
    if servers.is_empty() {
        ui.label(RichText::new("Keine; nur Smart Explorer.").color(muted));
    }
    for name in servers {
        ui.horizontal_wrapped(|ui| {
            ui.label(&name);
            if ui
                .add_enabled(page.operation.is_none(), egui::Button::new("Entfernen"))
                .clicked()
            {
                page.start(Op::RemoveServer(name.clone()), "");
            }
        });
    }
    ui.horizontal_wrapped(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut page.server_text)
                .hint_text("Servername wie im Agenten konfiguriert")
                .desired_width(240.0),
        );
        let enabled = !page.server_text.trim().is_empty() && page.operation.is_none();
        if ui
            .add_enabled(enabled, egui::Button::new("Erlauben …"))
            .on_hover_text("Schwächt die Sperre: fragt über den Systemdialog.")
            .clicked()
        {
            let name = page.server_text.trim().to_string();
            page.server_text.clear();
            page.start(Op::AllowServer(name), "");
        }
    });
}

fn list_body(ui: &mut egui::Ui, page: &mut MasksPage, list: &ListSummary) {
    let idle = page.operation.is_none();
    let name = list.name.clone();
    ui.horizontal_wrapped(|ui| {
        let (label, op) = if list.enabled {
            ("Deaktivieren …", Op::Enable(list.id, false))
        } else {
            ("Aktivieren", Op::Enable(list.id, true))
        };
        if ui.add_enabled(idle, egui::Button::new(label)).clicked() {
            page.start(op, &name);
        }
        let (label, op) = if list.locked {
            ("Entsperren …", Op::Lock(list.id, false))
        } else {
            ("Sperren", Op::Lock(list.id, true))
        };
        if ui
            .add_enabled(idle, egui::Button::new(label))
            .on_hover_text("Eine gesperrte Liste können Agenten nicht erweitern.")
            .clicked()
        {
            page.start(op, &name);
        }
        if ui
            .add_enabled(idle, egui::Button::new("Löschen …"))
            .clicked()
        {
            page.start(Op::Delete(list.id), &name);
        }
    });
    super::super::theme::section(ui, "Schutzpfade");
    for scope in &list.scopes {
        ui.horizontal_wrapped(|ui| {
            ui.label(scope);
            if ui
                .add_enabled(idle, egui::Button::new("Freigeben …"))
                .clicked()
            {
                page.start(Op::RemoveScope(list.id, scope.clone()), &name);
            }
        });
    }
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(idle, egui::Button::new("Ordner wählen …"))
            .clicked()
        {
            page.pick_scope = Some((list.id, name.clone()));
        }
        ui.add(
            egui::TextEdit::singleline(&mut page.scope_text)
                .hint_text("oder Ziel: @nas:/pfad, sftp://…, share://…")
                .desired_width(260.0),
        );
        if ui
            .add_enabled(
                idle && !page.scope_text.trim().is_empty(),
                egui::Button::new("Hinzufügen"),
            )
            .clicked()
        {
            let target = std::mem::take(&mut page.scope_text);
            page.add_scope(list.id, &name, target.trim().to_string());
        }
    });
    super::super::theme::section(ui, "Einträge");
    for entry in &list.entries {
        entry_row(ui, page, list, entry);
    }
    ui.separator();
    ui.label(RichText::new("Neuer Eintrag").strong());
    if form::form_ui(ui, &mut page.add_form, ("mask_add", list.id.0), true) && idle {
        match page.add_form.entry() {
            Ok(entry) => {
                page.add_form.value.clear();
                page.start(Op::Add(list.id, entry), &name);
            }
            Err(error) => page.message = Some(Err(error)),
        }
    }
}

fn entry_row(ui: &mut egui::Ui, page: &mut MasksPage, list: &ListSummary, entry: &EntrySummary) {
    let idle = page.operation.is_none();
    let name = list.name.clone();
    ui.horizontal_wrapped(|ui| {
        ui.strong(&entry.label);
        ui.label(form::describe(entry));
        match page.revealed.get(&entry.id) {
            Some(value) => {
                ui.monospace(value);
                if ui.button("Verbergen").clicked() {
                    page.revealed.remove(&entry.id);
                }
            }
            None => {
                ui.monospace("••••••");
                if ui
                    .add_enabled(idle, egui::Button::new("Anzeigen"))
                    .clicked()
                {
                    page.start(Op::Reveal(list.id, entry.id), &name);
                }
            }
        }
        if ui
            .add_enabled(idle, egui::Button::new("Bearbeiten"))
            .clicked()
        {
            page.editing = Some((entry.id, EntryForm::from_summary(entry)));
        }
        if ui
            .add_enabled(idle, egui::Button::new("Entfernen …"))
            .clicked()
        {
            page.revealed.remove(&entry.id);
            page.start(Op::RemoveEntry(list.id, entry.id), &name);
        }
    });
    let editing = page.editing.as_mut().filter(|(id, _)| *id == entry.id);
    if let Some((id, edit)) = editing {
        let id = *id;
        let update =
            form::form_ui(ui, edit, ("mask_edit", id.0), false).then(|| edit.update(entry));
        if let Some(update) = update {
            page.editing = None;
            page.revealed.remove(&id);
            page.start(Op::Update(list.id, id, update), &name);
        }
    }
}
