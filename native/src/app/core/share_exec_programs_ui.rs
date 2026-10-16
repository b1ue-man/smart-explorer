//! Program list of an Exec grant in the desktop GUI, with the rules and
//! warnings of `se share grants exec enable --allow-program/--full`.
//!
//! The draft lives in egui memory under the card's confirmation id (it
//! includes the policy revision, so a stored change starts a fresh draft).
//! An empty list is the full shell, exactly as the stored grant means it.
use super::{activation_warning, confirmation_id, ExecAction, ExecDeviceView};
use crate::app::theme;
use eframe::egui::{self, RichText};

/// The rules of the CLI help (`cli/share/grants_exec.rs`, ENABLE_HELP).
const RULES: &str = "Ein Eintrag ist ein Programmname (gesucht nur im festen Systemsuchpfad dieses Hosts, nie im PATH des anderen Geraets) oder ein absoluter Pfad. Gestartet wird nur dieselbe Datei unter demselben Namen; ein Eintrag, den dieser Host nicht findet, erlaubt nichts. Das Geraet darf nur LANG, LC_* und TERM als Umgebung senden; jede andere Variable (zum Beispiel PATH, LD_PRELOAD oder GIT_PAGER) lehnt den Start ab. Die Argumente bleiben frei: Shells, env, Interpreter (zum Beispiel python, perl, node) und Programme, die Befehle ausfuehren (zum Beispiel find, git, ssh, xargs, awk, sudo), sind eine volle Shell und gehoeren nicht in die Liste. Keine Sandbox.";

/// What a draft changes against the stored list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProgramChange {
    Unchanged,
    /// The list is emptied: the device gets the full shell again.
    ToFullShell,
    /// The full shell becomes a list.
    ToList,
    /// Another list.
    ListChanged,
}

pub(super) fn change(current: &[String], draft: &[String]) -> ProgramChange {
    match (current == draft, current.is_empty(), draft.is_empty()) {
        (true, _, _) => ProgramChange::Unchanged,
        (false, false, true) => ProgramChange::ToFullShell,
        (false, true, false) => ProgramChange::ToList,
        _ => ProgramChange::ListChanged,
    }
}

/// Adds the trimmed `input` with the checks of `ExecGrantSettings::validate`
/// (bare name or absolute path, no NUL, at most 4096 bytes); a duplicate is
/// refused.
pub(super) fn add_program(draft: &mut Vec<String>, input: &str) -> Result<(), String> {
    let program = input.trim();
    if program.is_empty() {
        return Err("Bitte einen Programmnamen oder absoluten Pfad eingeben.".into());
    }
    if draft.iter().any(|entry| entry == program) {
        return Err(format!("{program} steht schon in der Liste."));
    }
    crate::share::ExecGrantSettings {
        run_as: crate::share::ExecRunAs::Standard,
        programs: vec![program.to_string()],
    }
    .validate()?;
    draft.push(program.to_string());
    Ok(())
}

fn draft_id(view: &ExecDeviceView) -> egui::Id {
    confirmation_id(view, "programs")
}

/// The list editor: entries with „Entfernen“, a field with „Hinzufuegen“,
/// „Volle Shell (Liste leeren)“ and the rules. Returns the current draft.
pub(super) fn list_editor(ui: &mut egui::Ui, view: &ExecDeviceView) -> Vec<String> {
    let id = draft_id(view);
    let mut draft = ui.data_mut(|data| {
        data.get_temp::<Vec<String>>(id)
            .unwrap_or_else(|| view.programs.clone())
    });
    let input_id = confirmation_id(view, "program_input");
    let mut input = ui.data_mut(|data| data.get_temp::<String>(input_id).unwrap_or_default());
    let error_id = egui::Id::new(("share_exec_program_error", id));
    let mut changed = false;
    ui.label(RichText::new("Programmliste").strong());
    if draft.is_empty() {
        ui.label("Leer: volle Shell (beliebige Programme und Shell-Befehle, Terminal).");
    }
    let mut remove = None;
    for (index, program) in draft.iter().enumerate() {
        ui.horizontal_wrapped(|ui| {
            ui.monospace(program);
            if ui.small_button("Entfernen").clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        draft.remove(index);
        changed = true;
    }
    ui.horizontal_wrapped(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(&mut input)
                .hint_text("uptime oder /usr/bin/df")
                .desired_width(220.0),
        );
        if field.changed() {
            ui.data_mut(|data| data.insert_temp(input_id, input.clone()));
        }
        let submitted = field.lost_focus() && ui.input(|state| state.key_pressed(egui::Key::Enter));
        if ui.button("Hinzufuegen").clicked() || submitted {
            match add_program(&mut draft, &input) {
                Ok(()) => {
                    changed = true;
                    ui.data_mut(|data| {
                        data.remove::<String>(input_id);
                        data.remove::<String>(error_id);
                    });
                }
                Err(error) => ui.data_mut(|data| data.insert_temp(error_id, error)),
            }
        }
        if !draft.is_empty() && ui.button("Volle Shell (Liste leeren)").clicked() {
            draft.clear();
            changed = true;
        }
    });
    if let Some(error) = ui.data_mut(|data| data.get_temp::<String>(error_id)) {
        ui.colored_label(theme::danger(ui), error);
    }
    egui::CollapsingHeader::new("Regeln fuer Programmlisten")
        .id_salt(("share_exec_program_rules", id))
        .show(ui, |ui| {
            ui.add(egui::Label::new(RULES).wrap());
        });
    if changed {
        ui.data_mut(|data| data.insert_temp(id, draft.clone()));
    }
    draft
}

/// Editing the list of an enabled grant: like `enable` in the CLI every
/// change needs a confirmation, names the new reach and that the device's
/// running commands end.
pub(super) fn enabled_editor(
    ui: &mut egui::Ui,
    view: &ExecDeviceView,
    provider: &crate::share::ExecProviderStatus,
    action: &mut Option<ExecAction>,
) {
    egui::CollapsingHeader::new("Programmliste bearbeiten")
        .id_salt(("share_exec_program_editor", &view.target_key))
        .show(ui, |ui| {
            let draft = list_editor(ui, view);
            let kind = change(&view.programs, &draft);
            if kind == ProgramChange::Unchanged {
                return;
            }
            let warning = activation_warning(provider, view.run_as.account_name(), &draft);
            ui.colored_label(
                theme::danger(ui),
                RichText::new(&warning.full_access).strong(),
            );
            if let Some(elevated) = warning.elevated {
                ui.colored_label(theme::danger(ui), RichText::new(elevated).strong());
            }
            ui.label(match kind {
                ProgramChange::ToFullShell => {
                    "Die leere Liste gibt diesem Geraet wieder die volle Shell."
                }
                ProgramChange::ToList => {
                    "Dieses Geraet darf danach nur noch die gelisteten Programme starten, ohne Shell und Terminal."
                }
                _ => "Dieses Geraet darf danach genau die gelisteten Programme starten.",
            });
            ui.label("Laufende Befehle dieses Geraets enden wie bei jedem Rechtewechsel.");
            let settings = crate::share::ExecGrantSettings {
                run_as: view.run_as.clone(),
                programs: draft.clone(),
            };
            let invalid = settings.validate_for_this_host().err();
            if let Some(error) = &invalid {
                ui.colored_label(theme::danger(ui), error);
            }
            let confirm_id = confirmation_id(view, "programs_confirm");
            let mut confirmed =
                ui.data_mut(|data| data.get_temp::<bool>(confirm_id).unwrap_or_default());
            if ui
                .checkbox(&mut confirmed, "Ich bestaetige diese Aenderung bewusst")
                .changed()
            {
                ui.data_mut(|data| data.insert_temp(confirm_id, confirmed));
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        confirmed && invalid.is_none(),
                        egui::Button::new("Programmliste uebernehmen"),
                    )
                    .clicked()
                {
                    *action = Some((view.target.clone(), true, Some(settings.clone())));
                    discard(ui, view);
                }
                if ui.button("Verwerfen").clicked() {
                    discard(ui, view);
                }
            });
        });
}

fn discard(ui: &mut egui::Ui, view: &ExecDeviceView) {
    ui.data_mut(|data| {
        data.remove::<Vec<String>>(draft_id(view));
        data.remove::<bool>(confirmation_id(view, "programs_confirm"));
        data.remove::<String>(confirmation_id(view, "program_input"));
    });
}
