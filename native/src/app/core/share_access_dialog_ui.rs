//! The access dialog of one device (PRM-M9, C1, C6 B-a): the presets „Volle
//! Rechte als mein Konto“, „Eingeschränktes Fernzugriffskonto“ (when the
//! host offers se-remote) and „Nur bestimmte Programme“, the switch „Mit
//! Rückfrage“, „Ausführen als“, one row per capability of the catalog, the
//! period, the program list, and exactly one confirmation: the button that
//! names the consequence („Als {Konto} erlauben“). There is no checkbox to
//! tick first. The room dialog (`share_access_room_ui.rs`) uses the same
//! draft and controls. The draft lives in egui memory, one dialog at a time.
use std::collections::BTreeMap;

use eframe::egui::{self, RichText};

use super::programs_ui::{controls, notes};
use super::ExecDeviceView;
use crate::app::theme;
use crate::share::access_catalog::HelperState;
use crate::share::access_grant::{
    AccessGrant, AccessGrantChange, AccessPreset, AccountProfile, Capability, CapabilityDecision,
    Expiry,
};
use crate::share::ExecProviderStatus;

use AccessPreset::{FullOwn, Programs, RemoteAccount};
use CapabilityDecision::{Allow, Ask, Off};

/// The note of every change that narrows (C5).
pub(super) const NARROWING: &str =
    "Laufende Befehle dieses Geräts enden, wenn die Änderung Rechte einschränkt.";

/// One account „Ausführen als“ offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AccountOption {
    pub(super) profile: AccountProfile,
    pub(super) available: bool,
    /// Why it cannot be chosen, with the next step (C6).
    pub(super) reason: Option<String>,
}

/// How long the right lasts after the change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Period {
    /// The end the right has (or none).
    Keep,
    Unlimited,
    Hours(i64),
}

/// What the dialog shows and changes.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct AccessDraft {
    /// Target key of the device, or the room profile id of the room dialog.
    pub(super) key: String,
    pub(super) title: String,
    pub(super) revision: u64,
    pub(super) base: AccessGrant,
    pub(super) base_programs: Vec<String>,
    /// `None`: „Eigene Auswahl“.
    pub(super) preset: Option<AccessPreset>,
    pub(super) ask: bool,
    pub(super) account: AccountProfile,
    pub(super) decisions: BTreeMap<Capability, CapabilityDecision>,
    pub(super) programs: Vec<String>,
    pub(super) period: Period,
    pub(super) accounts: Vec<AccountOption>,
    pub(super) helper: HelperState,
    pub(super) account_input: String,
    /// Why the typed account name was not taken.
    pub(super) account_error: Option<String>,
    pub(super) program_input: String,
    /// Why the typed program was not added.
    pub(super) error: Option<String>,
}

impl AccessDraft {
    /// The right as it is. A right that allows nothing starts like `se share
    /// grants exec enable` (D.2.3): its account and program list stay,
    /// commands are allowed and the terminal too when there is no list.
    pub(super) fn of(
        key: &str,
        title: &str,
        revision: u64,
        (base, programs): (AccessGrant, Vec<String>),
        (accounts, helper): (Vec<AccountOption>, HelperState),
    ) -> Self {
        let ask = base.capabilities().any(|(_, decision)| decision == Ask);
        let mut draft = Self {
            key: key.into(),
            title: title.into(),
            revision,
            decisions: base.capabilities().collect(),
            account: base.account.clone(),
            programs: programs.clone(),
            base_programs: programs,
            ask,
            preset: None,
            period: Period::Keep,
            base,
            accounts,
            helper,
            account_input: String::new(),
            account_error: None,
            program_input: String::new(),
            error: None,
        };
        if !draft.base.grants_any() {
            let terminal = if draft.programs.is_empty() {
                Allow
            } else {
                Off
            };
            draft.decisions.insert(Capability::ExecCommands, Allow);
            draft.decisions.insert(Capability::ExecTerminal, terminal);
        }
        draft.preset = draft.detect();
        draft
    }

    pub(super) fn decision(&self, capability: Capability) -> CapabilityDecision {
        self.decisions.get(&capability).copied().unwrap_or_default()
    }

    /// The preset the draft equals; `None` for „Eigene Auswahl“.
    fn detect(&self) -> Option<AccessPreset> {
        let on = if self.ask { Ask } else { Allow };
        let screen = [Capability::ScreenView, Capability::ScreenControl];
        if self.decision(Capability::ExecCommands) != on
            || screen.iter().any(|c| self.decision(*c).is_on())
        {
            return None;
        }
        let terminal = self.decision(Capability::ExecTerminal);
        match (&self.account, terminal, self.programs.is_empty()) {
            (AccountProfile::Own, terminal, true) if terminal == on => Some(FullOwn),
            (AccountProfile::Remote, terminal, true) if terminal == on => Some(RemoteAccount),
            (AccountProfile::Own, Off, false) => Some(Programs),
            _ => None,
        }
    }

    pub(super) fn select(&mut self, preset: AccessPreset) {
        let change = preset.change(self.programs.clone(), self.ask);
        self.account = change.account.unwrap_or_default();
        self.decisions.extend(change.capabilities);
        self.programs = change.programs.unwrap_or_default();
        self.preset = Some(preset);
    }

    /// „Mit Rückfrage“: every allowed capability asks, or asks no more.
    pub(super) fn set_ask(&mut self, ask: bool) {
        self.ask = ask;
        for decision in self.decisions.values_mut() {
            *decision = match (*decision, ask) {
                (Allow, true) => Ask,
                (Ask, false) => Allow,
                (other, _) => other,
            };
        }
        self.preset = self.detect();
    }

    pub(super) fn set_decision(&mut self, capability: Capability, decision: CapabilityDecision) {
        self.decisions.insert(capability, decision);
        self.ask = self.decisions.values().any(|decision| *decision == Ask);
        self.preset = self.detect();
    }

    pub(super) fn set_account(&mut self, account: AccountProfile) {
        self.account = account;
        self.preset = self.detect();
    }

    /// A program list allows no terminal (C1).
    pub(super) fn set_programs(&mut self, programs: Vec<String>) {
        if !programs.is_empty() {
            self.decisions.insert(Capability::ExecTerminal, Off);
        }
        self.programs = programs;
        self.preset = self.detect();
    }

    /// Everything the dialog shows; the end only when a period was chosen.
    pub(super) fn change(&self, now: i64) -> AccessGrantChange {
        let expiry = match self.period {
            Period::Keep => None,
            Period::Unlimited => Some(Expiry::None),
            Period::Hours(hours) => Some(Expiry::At(now.saturating_add(hours * 3_600))),
        };
        AccessGrantChange {
            account: Some(self.account.clone()),
            capabilities: self.decisions.clone(),
            programs: Some(self.programs.clone()),
            expiry,
        }
    }

    /// The right after the change, checked as the worker checks it.
    pub(super) fn result(&self, now: i64) -> Result<(AccessGrant, Vec<String>), String> {
        let (mut grant, mut programs) = (self.base.clone(), self.base_programs.clone());
        grant.apply(&self.change(now), &mut programs)?;
        Ok((grant, programs))
    }

    /// The one confirmation names the consequence (C6).
    pub(super) fn confirm_label(&self, own_label: &str) -> String {
        if self.decisions.values().any(|decision| decision.is_on()) {
            format!("Als {} erlauben", self.account.label(own_label))
        } else {
            "Zugriff entziehen".into()
        }
    }

    /// Why the confirmation cannot be given now; `None` when it can.
    pub(super) fn blocker(
        &self,
        provider: &ExecProviderStatus,
        active: bool,
        now: i64,
    ) -> Option<String> {
        if let Some(problem) = self.problem(provider, active, now) {
            return Some(problem);
        }
        let unchanged =
            self.result(now).ok() == Some((self.base.clone(), self.base_programs.clone()));
        unchanged.then(|| "Keine Änderung.".into())
    }

    /// `blocker` without „Keine Änderung“ (the room dialog has no base).
    pub(super) fn problem(
        &self,
        provider: &ExecProviderStatus,
        active: bool,
        now: i64,
    ) -> Option<String> {
        if !active {
            return Some(
                "Die Freigabe dieses Geräts ist inaktiv; zuerst die Freigabe wieder erlauben."
                    .into(),
            );
        }
        if self.preset == Some(Programs) && self.programs.is_empty() {
            let text = "Für „Nur bestimmte Programme“ mindestens ein Programm in die Programmliste eintragen.";
            return Some(text.into());
        }
        let grant = match self.result(now) {
            Ok((grant, _)) => grant,
            Err(error) => return Some(error),
        };
        if grant.grants_exec() && !activation_ready(true, provider, active) {
            return Some(format!(
                "Befehle sind auf diesem Rechner nicht verfügbar: {}",
                provider.detail
            ));
        }
        let chosen = self
            .accounts
            .iter()
            .find(|option| option.profile == self.account);
        let unavailable = chosen.filter(|option| !option.available && grant.grants_exec());
        let reason = unavailable.map(|option| option.reason.clone());
        reason.map(|reason| reason.unwrap_or_else(|| "Dieses Konto ist nicht verfügbar.".into()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ExecWarning {
    pub(super) full_access: String,
    pub(super) elevated: Option<&'static str>,
}

pub(super) fn exec_warning(provider: &ExecProviderStatus) -> ExecWarning {
    ExecWarning {
        full_access: format!("FULL {} CODE EXECUTION", provider.user_label),
        elevated: provider.elevated.then_some(
            if provider.user_label.eq_ignore_ascii_case("root") {
                "REMOTE ROOT SHELL"
            } else {
                "REMOTE ADMINISTRATOR SHELL"
            },
        ),
    }
}

/// The warning of an access change for the chosen account and list: the
/// full shell, or the listed programs; the elevated shell warning whenever
/// the device runs as the (elevated) own account.
pub(super) fn activation_warning(
    provider: &ExecProviderStatus,
    account: Option<&str>,
    programs: &[String],
) -> ExecWarning {
    let named = account.filter(|account| !account.is_empty());
    let label = named.unwrap_or(provider.user_label.as_str());
    let base = exec_warning(provider);
    ExecWarning {
        full_access: if programs.is_empty() {
            format!("FULL {label} CODE EXECUTION")
        } else {
            format!("{label} EXECUTION OF {}", programs.join(", "))
        },
        elevated: if named.is_some() { None } else { base.elevated },
    }
}

/// The confirmation is possible: given, Exec available on this host, and
/// the device's base authorization active.
pub(super) fn activation_ready(
    confirmed: bool,
    provider: &ExecProviderStatus,
    base_authorized: bool,
) -> bool {
    confirmed && provider.available && base_authorized
}

/// What the device dialog asks the page to do.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum DialogAction {
    Apply {
        key: String,
        change: AccessGrantChange,
        revision: u64,
    },
}

fn dialog_id() -> egui::Id {
    egui::Id::new("share_access_dialog")
}

pub(super) fn open(ctx: &egui::Context, draft: AccessDraft) {
    ctx.data_mut(|data| data.insert_temp(dialog_id(), draft));
}

pub(super) fn current(ctx: &egui::Context) -> Option<AccessDraft> {
    ctx.data_mut(|data| data.get_temp::<AccessDraft>(dialog_id()))
}

fn close(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<AccessDraft>(dialog_id()));
}

/// The dialog of the device whose draft is open; `views` are the devices as
/// they are now. A grant changed meanwhile asks to reload first.
pub(super) fn show(
    ctx: &egui::Context,
    provider: &ExecProviderStatus,
    views: &[ExecDeviceView],
    now: i64,
) -> Option<DialogAction> {
    let mut draft = current(ctx)?;
    let Some(view) = views.iter().find(|view| view.target_key == draft.key) else {
        close(ctx);
        return None;
    };
    let (mut visible, mut done, mut action) = (true, false, None);
    egui::Window::new(format!("Zugriff für {}", draft.title))
        .id(dialog_id())
        .collapsible(false)
        .resizable(true)
        .default_width(480.0)
        .open(&mut visible)
        .show(ctx, |ui| {
            if view.policy_revision != draft.revision {
                ui.colored_label(theme::warning(ui), "Das Recht wurde inzwischen geändert.");
                if ui.button("Neu laden").clicked() {
                    let accounts = (draft.accounts.clone(), draft.helper);
                    draft = AccessDraft::of(
                        &view.target_key,
                        &view.device_name,
                        view.policy_revision,
                        (view.access.clone(), view.programs.clone()),
                        accounts,
                    );
                }
                return;
            }
            let other_accounts = crate::share::exec_run_as_supported();
            controls(ui, &mut draft, &provider.user_label, other_accounts);
            ui.separator();
            for note in notes(&draft, &draft.title, provider, now) {
                ui.colored_label(theme::danger(ui), note);
            }
            let blocker = draft.blocker(provider, view.base_authorized, now);
            if let Some(reason) = &blocker {
                ui.label(RichText::new(reason).color(theme::muted(ui)));
            }
            ui.horizontal_wrapped(|ui| {
                let label = RichText::new(draft.confirm_label(&provider.user_label)).strong();
                if ui
                    .add_enabled(blocker.is_none(), egui::Button::new(label))
                    .clicked()
                {
                    let (key, revision) = (draft.key.clone(), draft.revision);
                    action = Some(DialogAction::Apply {
                        key,
                        change: draft.change(now),
                        revision,
                    });
                    done = true;
                }
                done |= ui.button("Abbrechen").clicked();
            });
        });
    if done || !visible {
        close(ctx);
    } else {
        open(ctx, draft);
    }
    action
}
