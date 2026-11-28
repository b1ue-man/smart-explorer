use super::*;

#[test]
fn batches_task_settings_preserve_each_legacy_share_tab_destination() {
    let expected = [
        SettingsPage::Devices,
        SettingsPage::Rooms,
        SettingsPage::ThisDevice,
        SettingsPage::Diagnostics,
        SettingsPage::Connections,
        SettingsPage::Diagnostics,
        SettingsPage::Devices,
        SettingsPage::Devices,
        SettingsPage::ThisDevice,
        SettingsPage::Devices,
    ];
    for (tab, page) in expected.into_iter().enumerate() {
        assert_eq!(for_share_tab(tab), page, "legacy tab {tab}");
    }
    assert_eq!(for_share_tab(usize::MAX), SettingsPage::Diagnostics);
    assert_eq!(canonical(SettingsPage::Storage), SettingsPage::Diagnostics);
    assert_eq!(default_share_tab(SettingsPage::Storage), Some(3));
}

#[test]
fn batches_task_settings_fourteen_areas_roundtrip_without_reassigning_tabs() {
    let expected = [
        SettingsPage::ThisDevice,
        SettingsPage::Devices,
        SettingsPage::Rooms,
        SettingsPage::Connections,
        SettingsPage::Sync,
        SettingsPage::Vaults,
        SettingsPage::Drives,
        SettingsPage::Integration,
        SettingsPage::Masks,
        SettingsPage::Appearance,
        SettingsPage::Languages,
        SettingsPage::Updates,
        SettingsPage::Background,
        SettingsPage::Diagnostics,
    ];
    assert_eq!(AREAS.map(|(page, _)| page), expected);
    for page in expected {
        assert_eq!(canonical(page), page);
        if let Some(tab) = default_share_tab(page) {
            assert_eq!(for_share_tab(tab), page);
        } else {
            assert!(matches!(
                page,
                SettingsPage::Sync
                    | SettingsPage::Vaults
                    | SettingsPage::Drives
                    | SettingsPage::Integration
                    | SettingsPage::Masks
                    | SettingsPage::Appearance
                    | SettingsPage::Languages
                    | SettingsPage::Updates
                    | SettingsPage::Background
            ));
        }
    }
    assert_eq!(default_share_tab(SettingsPage::ThisDevice), Some(8));
}

use crate::app::settings_ui::settings_masks::{
    describe, level_text, work_summary, Level, Op, TestEntryForm as EntryForm,
};
use crate::masks::{EntryId, EntryKind, EntrySummary, ListId, Tolerance};

fn fuzzy_summary(tolerance: Tolerance) -> EntrySummary {
    EntrySummary {
        id: EntryId(4),
        kind: EntryKind::Fuzzy,
        label: "Gericht".into(),
        case_sensitive: false,
        whole_word: false,
        tolerance,
    }
}

#[test]
fn masks_lock_task_settings_page_is_an_area_of_its_own() {
    let (_, label) = AREAS
        .iter()
        .find(|(page, _)| *page == SettingsPage::Masks)
        .unwrap();
    assert_eq!(*label, "Maskenlisten");
    assert_eq!(canonical(SettingsPage::Masks), SettingsPage::Masks);
    assert_eq!(default_share_tab(SettingsPage::Masks), None);
}

#[test]
fn masks_lock_task_settings_form_levels_numbers_and_updates() {
    let mut form = EntryForm::default();
    assert!(form.entry().is_err(), "a value is required");
    form.kind = EntryKind::Fuzzy;
    form.value = "Schweinsbraten".into();
    form.level = Level::Numbers;
    form.trim = 3;
    form.inner = 1.5;
    form.label = " Gericht ".into();
    let entry = form.entry().unwrap();
    assert_eq!(
        entry.tolerance,
        Tolerance::Custom {
            trim: 3,
            inner_tenths: 15
        }
    );
    assert_eq!(entry.label.as_deref(), Some("Gericht"));
    assert!(!entry.case_sensitive && !entry.whole_word);
    let custom = fuzzy_summary(Tolerance::Custom {
        trim: 3,
        inner_tenths: 15,
    });
    let back = EntryForm::from_summary(&custom);
    assert_eq!(
        (back.level, back.trim, back.tolerance()),
        (Level::Numbers, 3, custom.tolerance)
    );
    assert!(back.value.is_empty(), "a summary carries no value");
    // Unchanged form: no field in the update; raised level only changes the tolerance.
    let unchanged = back.update(&custom);
    assert_eq!(unchanged, crate::masks::EntryUpdate::default());
    let mut higher = back.clone();
    higher.level = Level::High;
    let update = higher.update(&custom);
    assert_eq!(update.tolerance, Some(Tolerance::High));
    assert_eq!(update.value, None);
    assert_eq!(
        level_text(Tolerance::Custom {
            trim: 2,
            inner_tenths: 4
        }),
        "Kürzung 2, innen 0,4"
    );
    assert_eq!(
        describe(&fuzzy_summary(Tolerance::Low)),
        "unscharf · Toleranz gering"
    );
}

#[test]
fn masks_lock_task_settings_changes_that_refresh_the_hardening() {
    let list = ListId(1);
    for op in [
        Op::AddScope(list, "/p".into()),
        Op::RemoveScope(list, "/p".into()),
        Op::Enable(list, false),
        Op::Delete(list),
    ] {
        assert!(op.touches_scopes(), "{op:?}");
    }
    for op in [
        Op::Lock(list, true),
        Op::Reveal(list, EntryId(1)),
        Op::Create("x".into()),
        Op::Harden,
    ] {
        assert!(!op.touches_scopes(), "{op:?}");
    }
    assert_eq!(
        Op::Lock(list, false).reason("Kunden"),
        "Smart Explorer: Maskenliste „Kunden“ entsperren"
    );
    let applied = serde_json::json!({"hardening": {"state": "applied"}});
    assert_eq!(
        work_summary(&applied),
        "Härtung in Claude Code und Codex verankert."
    );
    let failed = serde_json::json!({"hardening": {"state": "failed", "error": "abgelehnt"}});
    assert_eq!(work_summary(&failed), "Härtung nicht verankert: abgelehnt");
}
