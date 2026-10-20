//! Acceptance signals of VLT-M10 (U29) on the calculation models of the vault
//! windows of the desktop: the rows of the sidebar, unlocking, locking and the
//! lock events. No window is drawn here, the models decide what the windows
//! show (U29-1). The names are those of `strang-tresore.md` T 533 and
//! `festlegungen.md` U29-12.
use super::vault_models::{
    apply_lock_event, failure_text, lock_call, lock_step, recovery_page, row_hover, row_text, rows,
    unlock_outcome, EventModel, Failure, LockCall, LockStep, RecordModel, RowState, SiteModel,
    StatusModel, TabEffect, TabModel, UnlockDone, VaultFeedState,
};

const SITE_A: &str = "0123456789abcdef0123456789abcdef";
const SITE_B: &str = "fedcba9876543210fedcba9876543210";

fn failure(code: Option<&str>, message: &str) -> Failure {
    Failure {
        code: code.map(str::to_string),
        message: message.to_string(),
    }
}

fn site_model(site: &str, state: &str) -> SiteModel {
    SiteModel {
        site: site.into(),
        label: format!("Tresor {site}"),
        state: state.into(),
        ..SiteModel::default()
    }
}

fn record(id: &str, label: &str) -> RecordModel {
    RecordModel {
        id: id.into(),
        label: label.into(),
        location: format!("/tresore/{label}"),
        vault_id: "vault-1".into(),
        ..RecordModel::default()
    }
}

fn event(seq: u64, site: &str, state: &str) -> EventModel {
    EventModel {
        seq,
        site: site.into(),
        state: state.into(),
        reason: None,
    }
}

// ─── Sidebar, unlock, lock ────────────────────────────────────────────────

#[test]
fn vault_task_gui_unlock_states() {
    // The four states of a row (U29-4): mark, title, hover text.
    assert_eq!(RowState::from_wire("locked"), RowState::Locked);
    assert_eq!(RowState::from_wire("unlocked"), RowState::Unlocked);
    assert_eq!(RowState::from_wire("locking"), RowState::Locking);
    assert_eq!(RowState::from_wire("unavailable"), RowState::Unavailable);
    assert_eq!(
        RowState::from_wire("from-a-newer-version"),
        RowState::Locked
    );
    assert_eq!(row_text(RowState::Locked, None), "Gesperrt");
    assert_eq!(row_text(RowState::Unlocked, None), "Entsperrt");
    assert_eq!(row_text(RowState::Locking, None), "Wird gesperrt …");
    assert_eq!(row_text(RowState::Unavailable, None), "Nicht erreichbar");
    assert_eq!(
        row_text(RowState::Unavailable, Some("Ordner fehlt")),
        "Nicht erreichbar: Ordner fehlt"
    );
    assert_eq!(
        row_text(RowState::Locked, Some("ignoriert")),
        "Gesperrt",
        "only an unavailable site names a reason"
    );
    assert_eq!(
        row_hover("Privat", RowState::Unlocked, None),
        "Privat – Entsperrt"
    );
    let marks: Vec<&str> = [
        RowState::Locked,
        RowState::Unlocked,
        RowState::Locking,
        RowState::Unavailable,
    ]
    .iter()
    .map(|state| state.icon())
    .collect();
    assert_eq!(marks, ["🔒", "🔓", "⏳", "⚠"]);

    // The answers of the unlock dialog (U29-5): every code of the plan.
    let expected = [
        ("vault_wrong_password", "Das Passwort ist falsch."),
        (
            "vault_unsupported_format",
            "Dieses Tresorformat wird nicht unterstützt.",
        ),
        (
            "vault_hub",
            "Dieser Tresor gehört zu einem Cryptomator Hub und lässt sich nicht mit einem Passwort entsperren.",
        ),
        (
            "vault_memory",
            "Der Arbeitsspeicher reicht zum Entsperren dieses Tresors nicht.",
        ),
        (
            "vault_no_saved_password",
            "Für diesen Tresor ist kein Passwort gespeichert.",
        ),
        ("vault_locked", "Der Tresor ist gesperrt."),
    ];
    for (code, text) in expected {
        let view = unlock_outcome(&Err(failure(Some(code), "Rohtext des Dienstes")));
        assert_eq!(view.error_line.as_deref(), Some(text), "{code}");
        assert!(view.keep_open, "{code}: the window stays open");
        assert!(view.clear_password, "{code}: the field is emptied");
        assert_eq!(view.notice, None);
    }
    // Every other code or none: the message of the service.
    for code in [None, Some("vault_unknown_in_this_build")] {
        let view = unlock_outcome(&Err(failure(code, "Verbindung verloren")));
        assert_eq!(view.error_line.as_deref(), Some("Verbindung verloren"));
        assert!(view.keep_open);
    }
    // Success closes the window; a password that could not be remembered is
    // a notice.
    let done = unlock_outcome(&Ok(UnlockDone::default()));
    assert_eq!(done.error_line, None);
    assert!(!done.keep_open);
    assert!(done.clear_password);
    assert_eq!(done.notice, None);
    let remembered = unlock_outcome(&Ok(UnlockDone {
        remember_error: Some("Schlüsselbund gesperrt".into()),
    }));
    assert_eq!(
        remembered.notice.as_deref(),
        Some("Passwort nicht gemerkt: Schlüsselbund gesperrt")
    );
    assert!(!remembered.keep_open);
    // No text of a code asks for a command line in a window.
    assert!(!failure_text(&failure(Some("vault_locked"), "")).contains("se vault"));
}

#[test]
fn vault_task_gui_rows_follow_the_list_and_the_service() {
    let records = [record(SITE_A, "Privat"), record(SITE_B, "Arbeit")];
    // Without a service every site is locked (V10).
    let all_locked = rows(None, &records);
    assert_eq!(all_locked.len(), 2);
    assert!(all_locked.iter().all(|row| row.state == RowState::Locked));
    assert_eq!(all_locked[0].label, "Privat");
    // With one, the state comes from it; a site only the service names (a
    // broken list entry) comes last.
    let status = StatusModel {
        sites: vec![
            site_model(SITE_B, "unlocked"),
            SiteModel {
                site: "kaputt".into(),
                state: "unavailable".into(),
                reason: Some("Datei nicht lesbar".into()),
                ..SiteModel::default()
            },
        ],
        ..StatusModel::default()
    };
    let listed = rows(Some(&status), &records);
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[0].state, RowState::Locked);
    assert_eq!(listed[1].state, RowState::Unlocked);
    assert_eq!(listed[2].site, "kaputt");
    assert_eq!(listed[2].label, "kaputt");
    assert_eq!(listed[2].state, RowState::Unavailable);
    assert_eq!(listed[2].reason.as_deref(), Some("Datei nicht lesbar"));
}

#[test]
fn vault_task_gui_sidebar_lock_event_clears_tab() {
    let prefix = |site: &str| Some(format!("vault://{site}"));
    let mut tabs = vec![
        TabModel { prefix: None },
        TabModel {
            prefix: prefix(SITE_A),
        },
        TabModel {
            prefix: prefix(SITE_B),
        },
        TabModel {
            prefix: Some("sftp://u@h:22".into()),
        },
        TabModel {
            prefix: prefix("Privat"),
        },
    ];
    // The tabs of the site, by its ID and by its label, are cleared; no
    // other tab is touched.
    let effects = apply_lock_event(&mut tabs, SITE_A, "Privat");
    assert_eq!(
        effects,
        vec![
            TabEffect::ClearWithNotice {
                tab: 1,
                text: "Tresor „Privat“ wurde gesperrt.".into()
            },
            TabEffect::ClearWithNotice {
                tab: 4,
                text: "Tresor „Privat“ wurde gesperrt.".into()
            },
        ]
    );
    assert_eq!(tabs[2].prefix, prefix(SITE_B));
    assert_eq!(tabs[3].prefix.as_deref(), Some("sftp://u@h:22"));
    // The same event clears nothing twice.
    assert!(apply_lock_event(&mut tabs, SITE_A, "Privat").is_empty());
    // A prefix that only starts like the site is another place.
    let mut other = vec![TabModel {
        prefix: Some(format!("vault://{SITE_A}0")),
    }];
    assert!(apply_lock_event(&mut other, SITE_A, "Privat").is_empty());

    // The feed finds the locks (U29-2): by event, by a state that fell from
    // unlocked, and after a restart of the service.
    let unlocked = |sites: &[(&str, &str)], events: Vec<EventModel>, event_seq: u64| StatusModel {
        sites: sites.iter().map(|(s, st)| site_model(s, st)).collect(),
        event_seq,
        events,
        ..StatusModel::default()
    };
    let records = [record(SITE_A, "Privat"), record(SITE_B, "Arbeit")];
    let mut feed = VaultFeedState::default();
    let poll = |feed: &mut VaultFeedState, status: Option<StatusModel>| {
        let _ = feed.apply(Ok(status), Ok(records.to_vec()));
        let rows = feed.rows();
        feed.newly_locked(&rows)
    };
    // Both unlocked: no lock.
    let both = unlocked(
        &[(SITE_A, "unlocked"), (SITE_B, "unlocked")],
        vec![event(1, SITE_A, "unlocked"), event(2, SITE_B, "unlocked")],
        2,
    );
    assert!(poll(&mut feed, Some(both)).is_empty());
    // A lock event: the site, once.
    let after = unlocked(
        &[(SITE_A, "locked"), (SITE_B, "unlocked")],
        vec![event(3, SITE_A, "locked")],
        3,
    );
    assert_eq!(
        poll(&mut feed, Some(after.clone())),
        vec![SITE_A.to_string()]
    );
    assert!(
        poll(&mut feed, Some(after)).is_empty(),
        "seen events are not new"
    );
    // The service goes away: every unlocked site counts as locked.
    assert_eq!(poll(&mut feed, None), vec![SITE_B.to_string()]);
    // A failed request keeps the last status (a hiccup is no lock).
    let mut steady = VaultFeedState::default();
    let up = unlocked(&[(SITE_A, "unlocked")], Vec::new(), 0);
    let _ = poll(&mut steady, Some(up));
    assert!(steady.apply(Err("Zeitüberschreitung".into()), Ok(records.to_vec())));
    let rows_after = steady.rows();
    assert_eq!(rows_after[0].state, RowState::Unlocked);
    assert!(steady.newly_locked(&rows_after).is_empty());
    assert_eq!(steady.error.as_deref(), Some("Zeitüberschreitung"));
    // A restarted service numbers its events from the start again.
    let mut restart = VaultFeedState::default();
    let high = unlocked(
        &[(SITE_A, "unlocked")],
        vec![event(40, SITE_A, "unlocked")],
        40,
    );
    let _ = poll(&mut restart, Some(high));
    let fresh = unlocked(&[(SITE_A, "unlocked")], vec![event(1, SITE_B, "locked")], 1);
    assert_eq!(poll(&mut restart, Some(fresh)), vec![SITE_B.to_string()]);
}

#[test]
fn vault_task_gui_unsaved_changes_dialog_lock_keep() {
    // The first attempt locks without keeping; the answer of the worker
    // decides.
    assert_eq!(
        lock_call(Some(SITE_A), false),
        LockCall {
            site: Some(SITE_A.into()),
            keep_unsaved: false
        }
    );
    let unsaved = Err(failure(Some("vault_unsaved_changes"), "ungespeichert"));
    assert_eq!(lock_step(&unsaved), LockStep::AskKeep);
    // The button „Sperren, Änderungen verschlüsselt aufbewahren“ calls
    // `lock_vaults(Some(site), true)`; „Abbrechen“ calls nothing.
    assert_eq!(
        lock_call(Some(SITE_A), true),
        LockCall {
            site: Some(SITE_A.into()),
            keep_unsaved: true
        }
    );
    assert_eq!(
        lock_call(None, true),
        LockCall {
            site: None,
            keep_unsaved: true
        }
    );
    assert_eq!(
        lock_step(&Ok(vec![SITE_A.to_string()])),
        LockStep::Done(vec![SITE_A.to_string()])
    );
    assert_eq!(
        lock_step(&Err(failure(None, "Dienst antwortet nicht"))),
        LockStep::Failed("Dienst antwortet nicht".into())
    );
}

#[test]
fn vault_task_gui_recovery_page_text_only() {
    let page = recovery_page();
    assert_eq!(page.inputs, 0, "no field for a recovery key (C1.10)");
    assert_eq!(page.buttons, 0, "no button on the page");
    assert_eq!(page.paragraphs.len(), 2);
    assert_eq!(page.paragraphs[0], crate::vault::recovery_key_info().note);
    assert!(page.paragraphs[0].contains("Wortliste"));
    assert!(page.paragraphs[1].contains("Cryptomator Desktop"));
}
