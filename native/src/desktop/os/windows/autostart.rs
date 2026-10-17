//! Login start of the se Desktop shell on Windows (C5.7): the value
//! `SmartExplorerDesktop` = `"<GUI>" --desktop-shell` under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, the pattern of the
//! sync daemon's `SmartExplorerSync`. A switch-off by the user under
//! `Explorer\StartupApproved\Run` is reported and never overwritten; se never
//! writes or deletes Windows' approval metadata.
use std::io;
use std::path::Path;

use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_BINARY};
use winreg::RegKey;

use crate::desktop::core::ledger::RUN_VALUE_NAME;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

/// The value data: the quoted GUI program and the shell switch.
pub(crate) fn run_command_for(gui: &Path) -> String {
    format!(
        "\"{}\" --desktop-shell",
        gui.to_string_lossy().replace('/', "\\")
    )
}

/// Writes (or refreshes) the Run value for `gui`.
pub(crate) fn write(gui: &Path) -> io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (run, _) = hkcu.create_subkey(RUN_KEY)?;
    run.set_value(RUN_VALUE_NAME, &run_command_for(gui))
}

/// Deletes exactly the value `name` (only this module's fixed name is ever
/// passed); every other value under `Run` stays.
pub(crate) fn remove(name: &str) -> io::Result<()> {
    if name != RUN_VALUE_NAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("fremder Autostart-Wert {name:?} wird nicht gelöscht"),
        ));
    }
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run = match hkcu.open_subkey_with_flags(RUN_KEY, KEY_READ | KEY_SET_VALUE) {
        Ok(run) => run,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    match run.delete_value(RUN_VALUE_NAME) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Whether the Run value exists (with any content).
pub(crate) fn present() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|run| run.get_raw_value(RUN_VALUE_NAME))
        .is_ok()
}

/// Windows was told to skip the entry (Task Manager or Settings
/// "Autostart"): the known disable states 3 and 7, as for the sync daemon.
pub(crate) fn disabled_by_system() -> bool {
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(APPROVED_KEY);
    let Ok(value) = key.and_then(|key| key.get_raw_value(RUN_VALUE_NAME)) else {
        return false;
    };
    if value.vtype != REG_BINARY || value.bytes.len() < 4 {
        return false;
    }
    let state = u32::from_le_bytes([
        value.bytes[0],
        value.bytes[1],
        value.bytes[2],
        value.bytes[3],
    ]);
    matches!(state, 3 | 7)
}
