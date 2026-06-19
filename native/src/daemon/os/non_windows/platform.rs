#[derive(Clone)]
pub struct DriveInfo {
    pub letter: String,
    pub label: String,
    pub serial: String,
}

pub(crate) fn removable_drives() -> Vec<DriveInfo> {
    Vec::new()
}

pub(crate) fn battery_saver_on() -> bool {
    false
}

pub(crate) fn on_metered_network() -> bool {
    false
}
