//! Acceptance signals of VLT-M10 (U29) on opening vault files and on the
//! windows that create, enter and manage vault sites: the open rule, the
//! stored setting „Beim Öffnen einhängen“, the checks of the creation wizard
//! and the lock settings.
use std::cell::Cell;
use std::path::PathBuf;

use super::vault_models::{open_plan, OpenPlan, RecordModel};
use super::vault_models_manage::{
    add_summary, create_location_issue, default_label, label_issue, parse_threshold, password_step,
    settings_change, LocationView, SettingsDraft, LOCATION_TAKEN, PASSWORDS_DIFFER,
};
use super::vault_ui_settings::{self, VaultUiSettings};

const SITE_A: &str = "0123456789abcdef0123456789abcdef";

fn record(id: &str, label: &str) -> RecordModel {
    RecordModel {
        id: id.into(),
        label: label.into(),
        location: format!("/tresore/{label}"),
        vault_id: "vault-1".into(),
        ..RecordModel::default()
    }
}

// ─── Opening files, settings ──────────────────────────────────────────────

#[test]
fn vault_task_gui_mount_on_open_is_async() {
    let location = "vault://0123456789abcdef0123456789abcdef/Plan.txt";
    let asked = Cell::new(0);
    let in_drive = |_: &str| -> Result<Option<PathBuf>, String> {
        asked.set(asked.get() + 1);
        Ok(Some(PathBuf::from(
            "/run/user/1000/smart-explorer/Tresore/A/Plan.txt",
        )))
    };
    let not_mounted = |_: &str| -> Result<Option<PathBuf>, String> {
        asked.set(asked.get() + 1);
        Ok(None)
    };
    let broken = |_: &str| -> Result<Option<PathBuf>, String> { Err("IPC".into()) };
    // Mounted: open at once. Not mounted: mount in the background, or ask for
    // a copy when the switch is off. The decision never mounts (the function
    // has no way to).
    assert_eq!(
        open_plan(location, true, &in_drive),
        OpenPlan::Immediate(PathBuf::from(
            "/run/user/1000/smart-explorer/Tresore/A/Plan.txt"
        ))
    );
    assert_eq!(open_plan(location, true, &not_mounted), OpenPlan::Mount);
    assert_eq!(
        open_plan(location, false, &not_mounted),
        OpenPlan::AskCopy("Der Tresor ist nicht als Laufwerk eingehängt".into())
    );
    assert_eq!(
        open_plan(location, true, &broken),
        OpenPlan::AskCopy("IPC".into())
    );
    assert_eq!(asked.get(), 3, "one quick request per decision");
}

#[test]
fn vault_task_gui_ui_settings_roundtrip() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("vaults").join("ui.json");
    // Nothing stored: the default, on (C7.2).
    assert_eq!(
        vault_ui_settings::load_from(&path),
        VaultUiSettings {
            mount_on_open: true
        }
    );
    // Off, and on again; the file has the schema of the plan.
    vault_ui_settings::save_to(
        &path,
        &VaultUiSettings {
            mount_on_open: false,
        },
    )
    .expect("save off");
    assert!(!vault_ui_settings::load_from(&path).mount_on_open);
    let text = std::fs::read_to_string(&path).expect("file");
    let value: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(value["schema"], 1);
    assert_eq!(value["mount_on_open"], false);
    vault_ui_settings::save_to(
        &path,
        &VaultUiSettings {
            mount_on_open: true,
        },
    )
    .expect("save on");
    assert!(vault_ui_settings::load_from(&path).mount_on_open);
    // A key of a newer version stays.
    crate::support_dirs::write_private_atomic(
        &path,
        br#"{"schema":2,"mount_on_open":false,"later":"x"}"#,
    )
    .expect("write");
    assert!(!vault_ui_settings::load_from(&path).mount_on_open);
    vault_ui_settings::save_to(
        &path,
        &VaultUiSettings {
            mount_on_open: true,
        },
    )
    .expect("save over");
    let kept: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("file")).expect("json");
    assert_eq!(kept["later"], "x");
    assert_eq!(kept["mount_on_open"], true);
    // Unreadable content is the default, never an error.
    crate::support_dirs::write_private_atomic(&path, b"kein json").expect("write");
    assert!(vault_ui_settings::load_from(&path).mount_on_open);
    assert!(vault_ui_settings::parse(r#"{"mount_on_open":"ja"}"#).mount_on_open);
    assert!(!vault_ui_settings::parse(r#"{"mount_on_open":false}"#).mount_on_open);
    assert!(vault_ui_settings::VaultUiSettings::default().mount_on_open);
}

#[test]
fn vault_task_gui_create_wizard_validation() {
    // Password too short, then different, then good (U29-6).
    let short = password_step("kurz", "kurz").expect_err("too short");
    assert!(short.contains("mindestens 8 Zeichen"), "{short}");
    assert_eq!(
        password_step("langes passwort", "anderes passwort"),
        Err(PASSWORDS_DIFFER.to_string())
    );
    assert_eq!(password_step("langes passwort", "langes passwort"), Ok(()));
    // The length counts characters after NFC, not bytes.
    assert_eq!(password_step("äöüäöüäö", "äöüäöüäö"), Ok(()));
    // A location that holds a vault is refused; others pass.
    let found = LocationView {
        is_vault: true,
        format: Some(8),
        key_source: Some("masterkeyfile".into()),
        problem: None,
    };
    assert_eq!(create_location_issue(&found), Some(LOCATION_TAKEN.into()));
    assert!(LOCATION_TAKEN.contains("Tresor hinzufügen…"));
    assert_eq!(create_location_issue(&LocationView::default()), None);
    // „Dateinamen kürzen ab“: 36 to 220, empty is the default.
    assert_eq!(parse_threshold(""), Ok(220));
    assert_eq!(parse_threshold("36"), Ok(36));
    assert_eq!(parse_threshold(" 220 "), Ok(220));
    for bad in ["35", "221", "abc", "-5", "1.5"] {
        assert!(parse_threshold(bad).is_err(), "{bad}");
    }
    // The label comes from the folder name; slash and empty are refused.
    assert_eq!(default_label("/home/anna/Tresore/Privat/"), "Privat");
    assert_eq!(default_label("C:\\Daten\\Arbeit"), "Arbeit");
    assert_eq!(default_label("sftp://u@h:22/safe"), "safe");
    assert_eq!(default_label(""), "Tresor");
    assert!(label_issue("  ").is_some());
    assert!(label_issue("a/b").is_some());
    assert!(label_issue("Privat").is_none());
    // „Tresor hinzufügen…“ names what it found.
    assert_eq!(
        add_summary(&LocationView::default()),
        ("Kein Tresor an diesem Ort".to_string(), false)
    );
    let (text, can_add) = add_summary(&found);
    assert_eq!(text, "Tresor Format 8, Schlüsseldatei");
    assert!(can_add);
    let hub = LocationView {
        is_vault: true,
        format: Some(8),
        key_source: Some("hub".into()),
        problem: Some("Hub".into()),
    };
    assert!(
        !add_summary(&hub).1,
        "a vault se cannot open is not entered"
    );
    // The settings page: only what differs, and wrong input is named.
    let mut base = record(SITE_A, "Privat");
    base.lock_on_sleep = true;
    let mut draft = SettingsDraft::from_record(&base);
    assert_eq!(
        settings_change(&draft, &base),
        Ok(crate::vault::SettingsChange::default())
    );
    draft.idle_on = true;
    draft.idle_text = "15".into();
    draft.lock_on_sleep = false;
    draft.label = "Privat neu".into();
    let change = settings_change(&draft, &base).expect("change");
    assert_eq!(change.auto_lock_idle_min, Some(Some(15)));
    assert_eq!(change.lock_on_sleep, Some(false));
    assert_eq!(change.label.as_deref(), Some("Privat neu"));
    assert_eq!(change.lock_on_screen_lock, None);
    draft.idle_text = "0".into();
    assert!(settings_change(&draft, &base).is_err());
    draft.idle_on = false;
    assert!(settings_change(&draft, &base).is_ok());
}
