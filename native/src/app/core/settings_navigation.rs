//! E12 navigation. Legacy settings variants and Share tab numbers retain
//! their own meaning; only their containing settings area is mapped here.
use super::settings_ui::SettingsPage;

pub(super) const AREAS: [(SettingsPage, &str); 12] = [
    (SettingsPage::ThisDevice, "Dieses Gerät"),
    (SettingsPage::Devices, "Geräte"),
    (SettingsPage::Rooms, "Räume"),
    (SettingsPage::Connections, "Verbindungen"),
    (SettingsPage::Sync, "Sync"),
    (SettingsPage::Vaults, "Tresore"),
    (SettingsPage::Drives, "Laufwerke"),
    (SettingsPage::Integration, "Integration"),
    (SettingsPage::Appearance, "Darstellung"),
    (SettingsPage::Updates, "Updates"),
    (SettingsPage::Background, "Hintergrund"),
    (SettingsPage::Diagnostics, "Diagnose und Protokolle"),
];

pub(super) fn for_share_tab(tab: usize) -> SettingsPage {
    match tab {
        0 | 6 | 7 | 9 => SettingsPage::Devices,
        1 => SettingsPage::Rooms,
        2 | 8 => SettingsPage::ThisDevice,
        4 => SettingsPage::Connections,
        3 | 5 => SettingsPage::Diagnostics,
        _ => SettingsPage::Diagnostics,
    }
}

pub(super) fn canonical(page: SettingsPage) -> SettingsPage {
    match page {
        SettingsPage::Storage => SettingsPage::Diagnostics,
        page => page,
    }
}

pub(super) fn default_share_tab(page: SettingsPage) -> Option<usize> {
    match canonical(page) {
        SettingsPage::ThisDevice => Some(8),
        SettingsPage::Devices => Some(0),
        SettingsPage::Rooms => Some(1),
        SettingsPage::Connections => Some(4),
        SettingsPage::Diagnostics => Some(3),
        _ => None,
    }
}
