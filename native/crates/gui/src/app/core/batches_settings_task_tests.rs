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
fn batches_task_settings_twelve_areas_roundtrip_without_reassigning_tabs() {
    let expected = [
        SettingsPage::ThisDevice,
        SettingsPage::Devices,
        SettingsPage::Rooms,
        SettingsPage::Connections,
        SettingsPage::Sync,
        SettingsPage::Vaults,
        SettingsPage::Drives,
        SettingsPage::Integration,
        SettingsPage::Appearance,
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
                    | SettingsPage::Appearance
                    | SettingsPage::Updates
                    | SettingsPage::Background
            ));
        }
    }
    assert_eq!(default_share_tab(SettingsPage::ThisDevice), Some(8));
}
