//! The controls of the access dialog (PRM-M9, C1): presets, „Mit
//! Rückfrage“, „Ausführen als“, one row per capability of the catalog, the
//! period and the program list with the rules of `se share grants access
//! --allow-program/--full`. The draft they change and the one confirmation
//! are in `share_access_dialog_ui.rs`. An empty program list is the full
//! shell, exactly as the stored grant means it.
use eframe::egui::{self, RichText};

use super::access_dialog::{activation_warning, AccessDraft, AccountOption, Period, NARROWING};
use super::ExecDeviceView;
use crate::app::theme;
use crate::share::access_catalog::{
    capability_catalog, CapabilityEntry, HelperState, HelperStatus, HostFacts,
};
use crate::share::access_grant::{AccessPreset, AccountProfile, Capability, CapabilityDecision};
use crate::share::exec_helper_protocol::AccountChoiceProfile;
use crate::share::ExecProviderStatus;

use AccessPreset::{FullOwn, Programs, RemoteAccount};
use CapabilityDecision::{Allow, Ask, Off};

/// The periods the dialog offers, in hours.
const PERIODS: [(i64, &str); 4] = [
    (1, "1 Stunde"),
    (8, "8 Stunden"),
    (24, "1 Tag"),
    (168, "7 Tage"),
];

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

/// What a program-list change means for the device (shown in the access
/// dialog before the one confirmation).
pub(super) fn change_text(kind: ProgramChange) -> Option<&'static str> {
    match kind {
        ProgramChange::Unchanged => None,
        ProgramChange::ToFullShell => {
            Some("Die leere Liste gibt diesem Gerät wieder die volle Shell.")
        }
        ProgramChange::ToList => Some(
            "Das Gerät darf danach nur noch die gelisteten Programme starten, ohne Shell und Terminal.",
        ),
        ProgramChange::ListChanged => {
            Some("Das Gerät darf danach genau die gelisteten Programme starten.")
        }
    }
}

/// The list editor of the access dialog: entries with „Entfernen“, a field
/// with „Hinzufügen“, „Volle Shell (Liste leeren)“ and the rules. `salt`
/// keeps the widget ids of one dialog apart; returns whether the list changed.
pub(super) fn list_editor(
    ui: &mut egui::Ui,
    salt: &str,
    draft: &mut Vec<String>,
    input: &mut String,
    error: &mut Option<String>,
) -> bool {
    let mut changed = false;
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
            egui::TextEdit::singleline(input)
                .hint_text("uptime oder /usr/bin/df")
                .desired_width(220.0),
        );
        let submitted = field.lost_focus() && ui.input(|state| state.key_pressed(egui::Key::Enter));
        if ui.button("Hinzufügen").clicked() || submitted {
            match add_program(draft, input) {
                Ok(()) => {
                    changed = true;
                    input.clear();
                    *error = None;
                }
                Err(message) => *error = Some(message),
            }
        }
        if !draft.is_empty() && ui.button("Volle Shell (Liste leeren)").clicked() {
            draft.clear();
            changed = true;
        }
    });
    if let Some(message) = error.as_deref() {
        ui.colored_label(theme::danger(ui), message);
    }
    egui::CollapsingHeader::new("Regeln für Programmlisten")
        .id_salt(("share_access_program_rules", salt))
        .show(ui, |ui| {
            ui.add(egui::Label::new(RULES).wrap());
        });
    changed
}

/// What the change means for `subject` (C6): reach and account, an
/// administrator account, a new program list, the end of running commands.
pub(super) fn notes(
    draft: &AccessDraft,
    subject: &str,
    provider: &ExecProviderStatus,
    now: i64,
) -> Vec<String> {
    let Ok((grant, programs)) = draft.result(now) else {
        return Vec::new();
    };
    let mut notes = Vec::new();
    let account = grant.account.label(&provider.user_label);
    let on = |capability| grant.decision(capability).is_on();
    let what = match (on(Capability::ExecCommands), on(Capability::ExecTerminal)) {
        (true, _) if !programs.is_empty() => {
            format!("nur die Programme {}", programs.join(", "))
        }
        (true, true) => "Programme, Shell-Befehle und ein Terminal".into(),
        (true, false) => "Programme und Shell-Befehle".into(),
        (false, true) => "ein Terminal".into(),
        (false, false) => String::new(),
    };
    if !what.is_empty() {
        notes.push(format!(
            "{subject} kann danach {what} mit den Rechten von {account} starten. Was \
             {account} im Betriebssystem nicht darf, darf auch das Gerät nicht."
        ));
        let named = grant.account.account_name();
        let warning = activation_warning(provider, named, &programs);
        let admin = named.is_some_and(|name| {
            name.eq_ignore_ascii_case("root") || name.eq_ignore_ascii_case("administrator")
        });
        if warning.elevated.is_some() || admin {
            let marker = warning.elevated.unwrap_or("REMOTE ADMINISTRATOR SHELL");
            notes.push(format!(
                "Achtung: {account} hat Administratorrechte; das Gerät steuert damit diesen \
                 Rechner vollständig ({}, {marker}).",
                warning.full_access
            ));
        }
    }
    if let Some(text) = change_text(change(&draft.base_programs, &programs)) {
        notes.push(text.to_string());
    }
    if !grant
        .narrowing_against(&draft.base, programs != draft.base_programs)
        .is_empty()
    {
        notes.push(NARROWING.to_string());
    }
    notes
}

/// „Ausführen als“ of this host: the worker's list (`Access::Accounts`)
/// with the German reasons and next steps of C6; without a running worker
/// the own account and the accounts grants name.
pub(super) fn account_options(views: &[ExecDeviceView]) -> (Vec<AccountOption>, HelperState) {
    let available = |profile| AccountOption {
        profile,
        available: true,
        reason: None,
    };
    let Ok((helper, choices)) = crate::daemon::access_accounts() else {
        let mut options = vec![available(AccountProfile::Own)];
        let named = views.iter().map(|view| &view.access.account);
        for account in named.filter(|account| account.account_name().is_some()) {
            if crate::share::exec_run_as_supported()
                && options.iter().all(|o| &o.profile != account)
            {
                options.push(available(account.clone()));
            }
        }
        return (options, HelperState::NotNeeded);
    };
    let options = choices.into_iter().filter_map(|choice| {
        let reason = super::accounts_ui::option_reason(&choice);
        let profile = match (choice.profile, choice.account) {
            (AccountChoiceProfile::Own, _) => AccountProfile::Own,
            (AccountChoiceProfile::Remote, _) => AccountProfile::Remote,
            (AccountChoiceProfile::Account, Some(name)) => AccountProfile::Account { name },
            (AccountChoiceProfile::Account, None) => return None,
        };
        Some(AccountOption {
            profile,
            available: choice.available,
            reason,
        })
    });
    (options.collect(), helper.state)
}

/// The capability rows of this host (S4 adds its rows to the catalog).
pub(super) fn catalog(helper: HelperState) -> Vec<CapabilityEntry> {
    let account_helper = HelperStatus {
        state: helper,
        version: None,
        detail: None,
    };
    capability_catalog(&HostFacts {
        graphical_session: crate::screen::graphical_session(),
        account_helper,
    })
}

/// Small grey heading of a part of a dialog (as on the share pages).
pub(super) fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).small().color(theme::muted(ui)));
}

fn account_name(profile: &AccountProfile, own_label: &str) -> String {
    match profile {
        AccountProfile::Own => format!("Mein Konto ({own_label})"),
        AccountProfile::Remote => "Fernzugriffskonto se-remote".into(),
        AccountProfile::Account { name } => format!("Konto {name}"),
    }
}

/// Presets, „Mit Rückfrage“, „Ausführen als“, the rows, the period and the
/// program list of `draft`.
pub(super) fn controls(
    ui: &mut egui::Ui,
    draft: &mut AccessDraft,
    own_label: &str,
    other_accounts: bool,
) {
    heading(ui, "VOREINSTELLUNG");
    let remote = draft
        .accounts
        .iter()
        .find(|o| o.profile == AccountProfile::Remote)
        .cloned();
    let presets = [
        (
            FullOwn,
            format!("Volle Rechte als mein Konto ({own_label})"),
            true,
        ),
        (
            RemoteAccount,
            "Eingeschränktes Fernzugriffskonto se-remote".into(),
            remote.as_ref().is_some_and(|o| o.available),
        ),
        (Programs, "Nur bestimmte Programme".into(), true),
    ];
    for (preset, label, enabled) in presets {
        if preset == RemoteAccount && remote.is_none() {
            continue;
        }
        let response = ui.add_enabled(
            enabled,
            egui::RadioButton::new(draft.preset == Some(preset), label),
        );
        if response.clicked() {
            draft.select(preset);
        }
    }
    if let Some(reason) = remote.and_then(|option| option.reason) {
        ui.small(reason);
    }
    if draft.preset.is_none() {
        ui.add_enabled(false, egui::RadioButton::new(true, "Eigene Auswahl"));
    }
    let mut ask = draft.ask;
    if ui
        .checkbox(
            &mut ask,
            "Mit Rückfrage: jeder Start wartet auf eine Bestätigung an diesem Gerät",
        )
        .changed()
    {
        draft.set_ask(ask);
    }
    heading(ui, "AUSFÜHREN ALS");
    egui::ComboBox::from_id_salt(("share_access_account", &draft.key))
        .selected_text(account_name(&draft.account, own_label))
        .show_ui(ui, |ui| {
            for option in draft.accounts.clone() {
                let label = account_name(&option.profile, own_label);
                let chosen = option.profile == draft.account;
                let response =
                    ui.add_enabled(option.available, egui::SelectableLabel::new(chosen, label));
                if response.clicked() {
                    draft.set_account(option.profile);
                }
            }
        });
    if other_accounts {
        ui.horizontal_wrapped(|ui| {
            ui.label("Anderes lokales Konto:");
            ui.add(egui::TextEdit::singleline(&mut draft.account_input).desired_width(140.0));
            if ui.button("Wählen").clicked() {
                let account = AccountProfile::Account {
                    name: draft.account_input.trim().into(),
                };
                draft.account_error = account.validate().err();
                if draft.account_error.is_none() {
                    draft.set_account(account);
                }
            }
        });
        if let Some(error) = &draft.account_error {
            ui.colored_label(theme::danger(ui), error);
        }
    }
    heading(ui, "RECHTE");
    for entry in catalog(draft.helper) {
        capability_row(ui, draft, &entry);
    }
    heading(ui, "GÜLTIG");
    period_choice(ui, draft);
    let title = format!("Programmliste ({})", draft.programs.len());
    egui::CollapsingHeader::new(title)
        .id_salt(("share_access_programs", &draft.key))
        .default_open(draft.preset == Some(Programs))
        .show(ui, |ui| {
            let (mut list, mut input, mut error) = (
                draft.programs.clone(),
                std::mem::take(&mut draft.program_input),
                draft.error.take(),
            );
            if list_editor(ui, &draft.key, &mut list, &mut input, &mut error) {
                draft.set_programs(list);
            }
            (draft.program_input, draft.error) = (input, error);
        });
}

fn capability_row(ui: &mut egui::Ui, draft: &mut AccessDraft, entry: &CapabilityEntry) {
    let listed = !draft.programs.is_empty() && entry.capability == Capability::ExecTerminal;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(entry.label).strong());
        for (decision, label) in [(Off, "Aus"), (Ask, "Fragen"), (Allow, "Erlauben")] {
            let selected = draft.decision(entry.capability) == decision;
            let enabled = decision == Off || !listed;
            if ui
                .add_enabled(enabled, egui::RadioButton::new(selected, label))
                .clicked()
            {
                draft.set_decision(entry.capability, decision);
            }
        }
    });
    let text = if listed {
        "Mit einer Programmliste gibt es kein Terminal."
    } else {
        entry.description
    };
    ui.label(RichText::new(text).small().color(theme::muted(ui)));
}

fn period_choice(ui: &mut egui::Ui, draft: &mut AccessDraft) {
    // „Ende nicht ändern“ keeps each grant's own end (none stays none).
    let keep = match draft.base.valid_until {
        Some(end) => format!("Bisheriges Ende behalten ({})", super::end_text(end)),
        None => "Ende nicht ändern".to_string(),
    };
    let label = |period: Period| match period {
        Period::Keep => keep.clone(),
        Period::Unlimited => "Unbegrenzt".to_string(),
        Period::Hours(hours) => PERIODS
            .iter()
            .find(|(h, _)| *h == hours)
            .map_or_else(|| format!("{hours} Stunden"), |(_, text)| text.to_string()),
    };
    let mut choices = vec![Period::Keep, Period::Unlimited];
    choices.extend(PERIODS.iter().map(|(hours, _)| Period::Hours(*hours)));
    egui::ComboBox::from_id_salt(("share_access_period", &draft.key))
        .selected_text(label(draft.period))
        .show_ui(ui, |ui| {
            for period in choices {
                ui.selectable_value(&mut draft.period, period, label(period));
            }
        });
}
