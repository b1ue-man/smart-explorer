//! „Konten dieses Geräts“ at the bottom of the tab „Befehle“ (PRM-M9, Z1):
//! the account service with state and version, every account of „Ausführen
//! als“ with the system's verdict (German reasons, C6), and the
//! administrator actions of `se share exec accounts` (set up with se-remote,
//! update, allow, take away, remove). An action asks with the system's
//! dialog (UAC, graphical polkit); where it cannot, the page shows the
//! command for an administrator with „Befehl kopieren“. The list is read in
//! the background (worker, else this process) on first show, after actions
//! and on „Aktualisieren“; it may run a logon probe per account.
use eframe::egui::{self, RichText};

use super::programs_ui::heading;
use crate::app::theme;
use crate::share::access_catalog::HelperState;
use crate::share::access_grant::{valid_account_name, REMOTE_ACCOUNT_NAME};
use crate::share::exec_helper_protocol::{
    AccountChoice, AccountChoiceProfile, AccountChoices, AccountReason,
};
use crate::share::{ExecAccountsAction, ExecAccountsOutcome};

#[path = "share_access_accounts_actions.rs"]
mod actions;
pub(in crate::app) use actions::ui_accounts;

/// C2: what sessions of another account cannot do on Windows (S4U logon
/// without password, session 0).
pub(super) const SESSION_LIMITS: &str = "In dieser Sitzung sind Netzlaufwerke und mit EFS \
     verschlüsselte Dateien nicht erreichbar (Anmeldung ohne Passwort). Programme mit Fenster \
     erscheinen nicht auf dem Bildschirm.";

/// What the page shows and the inputs of its forms.
#[derive(Clone, Debug, Default)]
pub(super) struct AccountsPage {
    pub(super) choices: Option<AccountChoices>,
    pub(super) reading: bool,
    /// The action that runs in the background.
    pub(super) busy: Option<&'static str>,
    pub(super) outcome: Option<(&'static str, ExecAccountsOutcome)>,
    pub(super) error: Option<String>,
    pub(super) setup_open: bool,
    pub(super) setup_remote: bool,
    pub(super) remove_open: bool,
    pub(super) remove_remote: bool,
    pub(super) account_input: String,
}

impl AccountsPage {
    /// A new page; setting up offers se-remote by default.
    pub(super) fn new() -> Self {
        Self {
            setup_remote: true,
            ..Self::default()
        }
    }
}

/// What a click on the page asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PageAction {
    Apply(ExecAccountsAction),
    CopyCommand(String),
    Refresh,
}

/// The German text of why an account cannot be chosen, or of its warning
/// (C6; CLI, JSON and the bridge show the platform's English `reason`).
pub(super) fn reason_text(choice: &AccountChoice) -> Option<String> {
    let name = choice.account.as_deref().unwrap_or(&choice.label);
    let detail = choice.reason.as_deref().unwrap_or_default();
    Some(match choice.reason_code? {
        AccountReason::NotFound => format!("Konto {name} gibt es auf diesem Gerät nicht."),
        AccountReason::Root => format!(
            "Konto {name} hat Administrator- bzw. root-Rechte; Befehle laufen darüber nie als \
             anderes Konto."
        ),
        AccountReason::NotAllowed => format!("Konto {name} ist für Fernzugriff nicht freigegeben."),
        AccountReason::HelperNotInstalled => "Der Kontodienst ist nicht eingerichtet.".into(),
        AccountReason::HelperOutdated => "Der Kontodienst braucht eine Aktualisierung.".into(),
        AccountReason::Unsupported => format!("Auf diesem Gerät nicht möglich ({detail})."),
        AccountReason::PasswordChangeRequired => format!(
            "Konto {name} verlangt einen Passwortwechsel; vorher ist kein Fernzugriff möglich."
        ),
        AccountReason::Expired => format!("Konto {name} ist abgelaufen oder gesperrt."),
        AccountReason::NoLoginShell => {
            format!("Warnung: Die Login-Shell von {name} führt keine Befehle aus.")
        }
        AccountReason::LogonRefused => format!(
            "Windows lehnt die Anmeldung ohne Passwort für {name} ab ({}).",
            windows_error(detail)
        ),
    })
}

/// The Windows error inside the English text of a refused logon.
fn windows_error(detail: &str) -> &str {
    match (detail.find('('), detail.rfind(')')) {
        (Some(open), Some(close)) if open < close => &detail[open + 1..close],
        _ => detail,
    }
}

/// The next step of an account that cannot be chosen, on this page.
pub(super) fn next_step_text(choice: &AccountChoice) -> Option<String> {
    let remote = choice.profile == AccountChoiceProfile::Remote;
    let step = match choice.reason_code? {
        AccountReason::NotAllowed => "„Freigeben“ (Administratorbestätigung)",
        AccountReason::HelperNotInstalled => "„Kontodienst einrichten…“",
        AccountReason::HelperOutdated => "„Kontodienst aktualisieren“",
        AccountReason::NotFound if remote => "„Fernzugriffskonto anlegen“",
        _ => return None,
    };
    Some(format!(
        "Nächster Schritt: {step} unter Konten dieses Geräts."
    ))
}

/// The reason and the next step of an account in one text, for the access
/// dialog's „Ausführen als“.
pub(super) fn option_reason(choice: &AccountChoice) -> Option<String> {
    match (reason_text(choice), next_step_text(choice)) {
        (Some(reason), Some(next)) => Some(format!("{reason} {next}")),
        (reason, next) => reason.or(next),
    }
}

fn account_title(choice: &AccountChoice) -> String {
    let name = choice.account.as_deref().unwrap_or(&choice.label);
    match choice.profile {
        AccountChoiceProfile::Own => format!("Mein Konto ({name})"),
        AccountChoiceProfile::Remote => format!("Fernzugriffskonto {REMOTE_ACCOUNT_NAME}"),
        AccountChoiceProfile::Account => format!("Konto {name}"),
    }
}

/// The page; `session_limits` adds the hint of C2 (Windows).
pub(super) fn page(
    ui: &mut egui::Ui,
    page: &mut AccountsPage,
    session_limits: bool,
) -> Option<PageAction> {
    heading(ui, "KONTEN DIESES GERÄTS");
    let mut action = None;
    if let Some(name) = page.busy {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(format!("Wird ausgeführt: {name} …"));
        });
    }
    let Some(choices) = page.choices.clone() else {
        ui.label(if page.reading {
            "Die Konten werden gelesen …"
        } else {
            "Die Konten sind noch nicht gelesen."
        });
        return outcome_block(ui, page).or(action);
    };
    let enabled = page.busy.is_none();
    action = action.or(service_block(ui, page, &choices, enabled));
    if session_limits && choices.helper.state != HelperState::NotInstalled {
        ui.label(
            RichText::new(SESSION_LIMITS)
                .small()
                .color(theme::muted(ui)),
        );
    }
    let managed = matches!(
        choices.helper.state,
        HelperState::Installed | HelperState::Outdated
    );
    for choice in &choices.choices {
        action = action.or(account_row(ui, choice, managed, enabled));
    }
    if managed {
        ui.horizontal_wrapped(|ui| {
            ui.label("Weiteres lokales Konto:");
            ui.add(egui::TextEdit::singleline(&mut page.account_input).desired_width(140.0));
            let name = page.account_input.trim().to_string();
            let valid = valid_account_name(&name);
            if ui
                .add_enabled(enabled && valid, egui::Button::new("Konto freigeben"))
                .clicked()
            {
                action = Some(PageAction::Apply(ExecAccountsAction::Allow {
                    account: name,
                }));
            }
        });
    }
    if ui
        .add_enabled(!page.reading, egui::Button::new("Aktualisieren"))
        .clicked()
    {
        action = Some(PageAction::Refresh);
    }
    outcome_block(ui, page).or(action)
}

/// The account service: its state and the actions it allows.
fn service_block(
    ui: &mut egui::Ui,
    page: &mut AccountsPage,
    choices: &AccountChoices,
    enabled: bool,
) -> Option<PageAction> {
    let helper = &choices.helper;
    let detail = helper.detail.clone().unwrap_or_default();
    let mut action = None;
    match helper.state {
        HelperState::NotNeeded => {
            ui.label("Kontowechsel ohne Kontodienst: der Worker dieses Geräts läuft als root.");
        }
        HelperState::Unavailable => {
            ui.label(format!(
                "Auf diesem Gerät laufen Befehle nur unter dem eigenen Konto ({detail})."
            ));
        }
        HelperState::NotInstalled => {
            ui.label("Der Kontodienst ist nicht eingerichtet.");
            if ui
                .add_enabled(enabled, egui::Button::new("Kontodienst einrichten…"))
                .clicked()
            {
                page.setup_open = !page.setup_open;
            }
        }
        HelperState::Installed | HelperState::Outdated => {
            if helper.state == HelperState::Installed {
                let version = helper.version.as_deref().unwrap_or("unbekannt");
                ui.label("Kontodienst eingerichtet.");
                egui::CollapsingHeader::new("Diagnose des Kontodiensts")
                    .id_salt("account_service_diag")
                    .show(ui, |ui| {
                        ui.monospace(format!("Version {version}"));
                    });
                if !detail.is_empty() {
                    ui.small(detail);
                }
            } else {
                let text = format!("Der Kontodienst braucht eine Aktualisierung: {detail}");
                ui.colored_label(theme::warning(ui), text);
            }
            ui.horizontal_wrapped(|ui| {
                let update = egui::Button::new("Kontodienst aktualisieren");
                if ui.add_enabled(enabled, update).clicked() {
                    action = Some(PageAction::Apply(ExecAccountsAction::Update));
                }
                let remove = egui::Button::new("Kontodienst entfernen…");
                if ui.add_enabled(enabled, remove).clicked() {
                    page.remove_open = !page.remove_open;
                }
            });
        }
    }
    if page.setup_open && helper.state == HelperState::NotInstalled {
        let text = "Der Kontodienst läuft mit Administratorrechten und startet Befehle anderer \
                    Geräte unter den Konten, die hier freigegeben sind. Was ein Konto im \
                    Betriebssystem nicht darf, darf auch das Gerät nicht. Das Einrichten fragt \
                    einmal nach einer Administratorbestätigung.";
        let option = "Fernzugriffskonto se-remote anlegen und freigeben";
        if confirm_form(
            ui,
            text,
            (&mut page.setup_remote, option),
            "Einrichten",
            enabled,
        ) {
            action = Some(PageAction::Apply(ExecAccountsAction::Setup {
                remote_account: page.setup_remote,
                allow: Vec::new(),
            }));
        }
    }
    if page.remove_open && helper.state != HelperState::NotInstalled {
        let text = "Entfernt den Kontodienst und seine Programmkopie; laufende Befehle unter \
                    anderen Konten enden.";
        let option = "Fernzugriffskonto se-remote mit seinen Dateien löschen";
        if confirm_form(
            ui,
            text,
            (&mut page.remove_remote, option),
            "Entfernen",
            enabled,
        ) {
            action = Some(PageAction::Apply(ExecAccountsAction::Remove {
                delete_remote_account: page.remove_remote,
            }));
        }
    }
    action
}

/// A framed text, one option and the button that confirms; `true` when
/// the button was clicked.
fn confirm_form(
    ui: &mut egui::Ui,
    text: &str,
    (checked, option): (&mut bool, &str),
    button: &str,
    enabled: bool,
) -> bool {
    let mut clicked = false;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(text);
        ui.checkbox(checked, option);
        clicked = ui.add_enabled(enabled, egui::Button::new(button)).clicked();
    });
    clicked
}

/// One account with its state and the action that fits it.
fn account_row(
    ui: &mut egui::Ui,
    choice: &AccountChoice,
    managed: bool,
    enabled: bool,
) -> Option<PageAction> {
    let mut action = None;
    let account = choice.account.clone().unwrap_or_default();
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(account_title(choice)).strong());
        if choice.available {
            ui.colored_label(theme::success(ui), "verfügbar");
        } else {
            ui.colored_label(theme::warning(ui), "nicht verfügbar");
        }
        if choice.profile == AccountChoiceProfile::Own || !managed {
            return;
        }
        let button = match choice.reason_code {
            Some(AccountReason::NotAllowed) => Some(("Freigeben", allow(&account))),
            Some(AccountReason::NotFound) if choice.profile == AccountChoiceProfile::Remote => {
                let setup = ExecAccountsAction::Setup {
                    remote_account: true,
                    allow: Vec::new(),
                };
                Some(("Fernzugriffskonto anlegen", setup))
            }
            Some(AccountReason::HelperOutdated) => None,
            _ => Some((
                "Freigabe entziehen",
                ExecAccountsAction::Disallow {
                    account: account.clone(),
                },
            )),
        };
        if let Some((label, apply)) = button {
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                action = Some(PageAction::Apply(apply));
            }
        }
    });
    if let Some(reason) = reason_text(choice) {
        ui.label(RichText::new(reason).small());
    }
    if let Some(next) = next_step_text(choice) {
        ui.label(RichText::new(next).small().color(theme::muted(ui)));
    }
    action
}

fn allow(account: &str) -> ExecAccountsAction {
    ExecAccountsAction::Allow {
        account: account.to_string(),
    }
}

/// The answer of the last action: its steps, or the command a person runs
/// as administrator with „Befehl kopieren“; else an error.
fn outcome_block(ui: &mut egui::Ui, page: &AccountsPage) -> Option<PageAction> {
    let mut action = None;
    if let Some(error) = &page.error {
        ui.colored_label(theme::danger(ui), error);
    }
    match &page.outcome {
        Some((name, ExecAccountsOutcome::Done(steps))) => {
            ui.colored_label(theme::success(ui), format!("{name}: erledigt"));
            for step in steps {
                ui.small(step);
            }
        }
        Some((_, ExecAccountsOutcome::NeedsAdministrator { command, reason })) => {
            let text = format!(
                "Administratorrechte nötig ({reason}). Diesen Befehl in einem Terminal als \
                 Administrator ausführen:"
            );
            ui.colored_label(theme::warning(ui), text);
            ui.horizontal_wrapped(|ui| {
                ui.monospace(command);
                if ui.button("Befehl kopieren").clicked() {
                    action = Some(PageAction::CopyCommand(command.clone()));
                }
            });
        }
        None => {}
    }
    action
}
