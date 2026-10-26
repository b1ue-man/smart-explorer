//! Acceptance of „Konten dieses Geräts“ (PRM-M9, R 391) in real egui frames
//! without a display, with the harness of `share_access_ui_tests.rs`: the
//! states of the account service and of the accounts, and the command a
//! person runs when no administrator dialog is possible.
use eframe::egui;

use super::access_ui_tests::Harness;
use super::accounts_ui::{option_reason, page, AccountsPage, PageAction, SESSION_LIMITS};
use crate::share::access_catalog::{HelperState, HelperStatus};
use crate::share::exec_helper_protocol::{
    AccountChoice, AccountChoiceProfile, AccountChoices, AccountReason,
};
use crate::share::{ExecAccountsAction, ExecAccountsOutcome};

fn choices(state: HelperState, detail: Option<&str>, others: Vec<AccountChoice>) -> AccountChoices {
    let mut choices = vec![AccountChoice::own("alice")];
    choices.extend(others);
    AccountChoices {
        helper: HelperStatus {
            state,
            version: (state == HelperState::Installed).then(|| "0.5.183".to_string()),
            detail: detail.map(str::to_string),
        },
        choices,
    }
}

fn remote(reason: Option<(AccountReason, &str)>) -> AccountChoice {
    let mut choice = AccountChoice::other(AccountChoiceProfile::Remote, "se-remote");
    if let Some((code, text)) = reason {
        choice.refuse(code, text.into(), None);
    }
    choice
}

fn deploy(reason: Option<(AccountReason, &str)>) -> AccountChoice {
    let mut choice = AccountChoice::other(AccountChoiceProfile::Account, "deploy");
    if let Some((code, text)) = reason {
        choice.refuse(code, text.into(), None);
    }
    choice
}

/// Draws `state` until it settles and runs `check` with the harness and
/// the drawing.
fn with_page(
    state: AccountsPage,
    session_limits: bool,
    check: impl FnOnce(&mut Harness, &mut dyn FnMut(&egui::Context) -> Option<PageAction>),
) {
    let mut h = Harness::new();
    let mut state = state;
    let mut draw = |ctx: &egui::Context| {
        let mut asked = None;
        egui::CentralPanel::default().show(ctx, |ui| {
            asked = page(ui, &mut state, session_limits);
        });
        asked
    };
    h.settle(&mut draw);
    check(&mut h, &mut draw);
}

fn shown(choices: AccountChoices) -> AccountsPage {
    AccountsPage {
        choices: Some(choices),
        ..AccountsPage::new()
    }
}

#[test]
fn perm_task_gui_accounts_page_states() {
    // Not set up: the service is offered with se-remote, one confirmation.
    let not_set_up = (AccountReason::HelperNotInstalled, "not set up");
    let state = shown(choices(
        HelperState::NotInstalled,
        None,
        vec![remote(Some(not_set_up))],
    ));
    with_page(state, true, |h, draw| {
        for label in [
            "KONTEN DIESES GERÄTS",
            "Der Kontodienst ist nicht eingerichtet.",
            "Mein Konto (alice)",
            "Fernzugriffskonto se-remote",
            "nicht verfügbar",
            "Nächster Schritt: „Kontodienst einrichten…“ unter Konten dieses Geräts.",
        ] {
            assert!(h.capture.contains(label), "{label}");
        }
        assert!(!h.capture.contains(SESSION_LIMITS), "only once set up");
        assert_eq!(h.click("Kontodienst einrichten…", 0, draw), None);
        assert!(h
            .capture
            .contains("Fernzugriffskonto se-remote anlegen und freigeben"));
        let setup = ExecAccountsAction::Setup {
            remote_account: true,
            allow: Vec::new(),
        };
        assert_eq!(
            h.click("Einrichten", 0, draw),
            Some(PageAction::Apply(setup))
        );
    });

    // Set up: version, update, remove; every listed account can be taken
    // away, and the Windows hint of C2 is shown.
    let state = shown(choices(
        HelperState::Installed,
        None,
        vec![remote(None), deploy(None)],
    ));
    with_page(state, true, |h, draw| {
        assert!(h.capture.contains("Kontodienst eingerichtet."));
        assert_eq!(h.click("Diagnose des Kontodiensts", 0, draw), None);
        assert!(h.capture.contains("Version 0.5.183"));
        assert!(h.capture.contains("verfügbar"));
        assert!(h.capture.contains(SESSION_LIMITS));
        let disallow = ExecAccountsAction::Disallow {
            account: "deploy".into(),
        };
        assert_eq!(
            h.click("Freigabe entziehen", 1, draw),
            Some(PageAction::Apply(disallow))
        );
        assert_eq!(
            h.click("Kontodienst aktualisieren", 0, draw),
            Some(PageAction::Apply(ExecAccountsAction::Update))
        );
        assert_eq!(h.click("Kontodienst entfernen…", 0, draw), None);
        let remove = ExecAccountsAction::Remove {
            delete_remote_account: false,
        };
        assert_eq!(
            h.click("Entfernen", 0, draw),
            Some(PageAction::Apply(remove))
        );
    });

    // Outdated: the reason and the update.
    let outdated = (AccountReason::HelperOutdated, "needs an update");
    let state = shown(choices(
        HelperState::Outdated,
        Some("its socket does not answer"),
        vec![remote(Some(outdated))],
    ));
    with_page(state, false, |h, draw| {
        assert!(h
            .capture
            .contains("Der Kontodienst braucht eine Aktualisierung: its socket does not answer"));
        assert!(h
            .capture
            .contains("Der Kontodienst braucht eine Aktualisierung."));
        assert_eq!(
            h.click("Kontodienst aktualisieren", 0, draw),
            Some(PageAction::Apply(ExecAccountsAction::Update))
        );
    });

    // An account whose logon probe failed: unavailable, with the Windows
    // error in the German text of C6.
    let refused = (
        AccountReason::LogonRefused,
        "Windows refuses the logon without password for deploy (The user name or password is \
         incorrect. (os error 1326))",
    );
    let failed = deploy(Some(refused));
    let expected = "Windows lehnt die Anmeldung ohne Passwort für deploy ab (The user name or \
                    password is incorrect. (os error 1326)).";
    assert_eq!(option_reason(&failed).as_deref(), Some(expected));
    let state = shown(choices(HelperState::Installed, None, vec![failed]));
    with_page(state, false, |h, _| {
        assert!(h.capture.contains("Konto deploy"));
        assert!(h.capture.contains("nicht verfügbar"));
        assert!(h.capture.contains(expected));
    });
}

#[test]
fn perm_task_gui_setup_without_pkexec_shows_sudo_command() {
    let command = "sudo /usr/bin/se share exec accounts setup --remote-account --yes";
    let outcome = ExecAccountsOutcome::NeedsAdministrator {
        command: command.into(),
        reason: "pkexec is not installed".into(),
    };
    let state = AccountsPage {
        outcome: Some(("Kontodienst einrichten", outcome)),
        ..shown(choices(HelperState::NotInstalled, None, Vec::new()))
    };
    with_page(state, false, |h, draw| {
        assert!(h.capture.contains(
            "Administratorrechte nötig (pkexec is not installed). Diesen Befehl in einem \
             Terminal als Administrator ausführen:"
        ));
        assert!(h.capture.contains(command));
        assert_eq!(
            h.click("Befehl kopieren", 0, draw),
            Some(PageAction::CopyCommand(command.into()))
        );
    });
}
