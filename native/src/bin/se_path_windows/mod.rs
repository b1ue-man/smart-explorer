//! Installer-only registration for the standalone `se.exe` command.

use std::io;
use std::path::Path;

use winreg::enums::{HKEY_CURRENT_USER, REG_EXPAND_SZ, REG_SZ};
use winreg::types::ToRegValue;
use winreg::{RegKey, RegValue};

const ENVIRONMENT_KEY: &str = "Environment";
const INSTALLER_KEY: &str = r"Software\SmartExplorer\Installer";
const OWNED_PATH: &str = "CliPath";
const OWNED_FLAG: &str = "CliPathOwned";

pub(crate) fn register() -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let directory = executable
        .parent()
        .ok_or_else(|| io::Error::other("se.exe has no installation directory"))?;
    let directory_text = path_text(directory)?;
    if directory_text.contains(';') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the se.exe installation path cannot contain a semicolon",
        ));
    }

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (installer, _) = hkcu.create_subkey(INSTALLER_KEY)?;
    remove_previous_owned_path(&hkcu, &installer, &directory_text)?;

    let (environment, _) = hkcu.create_subkey(ENVIRONMENT_KEY)?;
    let (path, value_type) = read_user_path(&environment)?;
    let (updated, added) = add_path_component(&path, &directory_text);
    if added {
        write_user_path(&environment, &updated, value_type)?;
    }
    let previously_owned = installer
        .get_value::<u32, _>(OWNED_FLAG)
        .unwrap_or_default()
        != 0
        && installer
            .get_value::<String, _>(OWNED_PATH)
            .is_ok_and(|path| same_component(&path, &directory_text));
    installer.set_value(OWNED_PATH, &directory_text)?;
    installer.set_value(OWNED_FLAG, &u32::from(added || previously_owned))?;

    broadcast_environment_change();
    Ok(())
}

pub(crate) fn unregister() -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let directory = executable
        .parent()
        .ok_or_else(|| io::Error::other("se.exe has no installation directory"))?;
    let directory_text = path_text(directory)?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    if let Ok(installer) = hkcu.open_subkey_with_flags(INSTALLER_KEY, winreg::enums::KEY_ALL_ACCESS)
    {
        let owned = installer
            .get_value::<u32, _>(OWNED_FLAG)
            .unwrap_or_default()
            != 0;
        let recorded = installer.get_value::<String, _>(OWNED_PATH).ok();
        if owned
            && recorded
                .as_deref()
                .is_some_and(|path| same_component(path, &directory_text))
        {
            remove_user_path(&hkcu, &directory_text)?;
        }
        ignore_missing(installer.delete_value(OWNED_PATH))?;
        ignore_missing(installer.delete_value(OWNED_FLAG))?;
    }
    broadcast_environment_change();
    Ok(())
}

fn remove_previous_owned_path(hkcu: &RegKey, installer: &RegKey, current: &str) -> io::Result<()> {
    let owned = installer
        .get_value::<u32, _>(OWNED_FLAG)
        .unwrap_or_default()
        != 0;
    let previous = installer.get_value::<String, _>(OWNED_PATH).ok();
    if owned {
        if let Some(previous) = previous.filter(|path| !same_component(path, current)) {
            remove_user_path(hkcu, &previous)?;
        }
    }
    Ok(())
}

fn remove_user_path(hkcu: &RegKey, component: &str) -> io::Result<()> {
    let (environment, _) = hkcu.create_subkey(ENVIRONMENT_KEY)?;
    let (path, value_type) = read_user_path(&environment)?;
    let (updated, removed) = remove_path_component(&path, component);
    if removed {
        write_user_path(&environment, &updated, value_type)?;
    }
    Ok(())
}

fn read_user_path(environment: &RegKey) -> io::Result<(String, winreg::enums::RegType)> {
    match environment.get_raw_value("Path") {
        Ok(raw) if raw.vtype == REG_SZ || raw.vtype == REG_EXPAND_SZ => {
            let value = environment.get_value::<String, _>("Path")?;
            Ok((value, raw.vtype))
        }
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the user Path registry value is not a string",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok((String::new(), REG_EXPAND_SZ)),
        Err(error) => Err(error),
    }
}

fn write_user_path(
    environment: &RegKey,
    path: &str,
    value_type: winreg::enums::RegType,
) -> io::Result<()> {
    let mut raw: RegValue = path.to_reg_value();
    raw.vtype = value_type;
    environment.set_raw_value("Path", &raw)
}

fn add_path_component(path: &str, component: &str) -> (String, bool) {
    if path
        .split(';')
        .any(|entry| same_component(entry, component))
    {
        return (path.to_string(), false);
    }
    if path.is_empty() {
        (component.to_string(), true)
    } else if path.ends_with(';') {
        (format!("{path}{component}"), true)
    } else {
        (format!("{path};{component}"), true)
    }
}

fn remove_path_component(path: &str, component: &str) -> (String, bool) {
    let mut removed = false;
    let entries = path.split(';').filter(|entry| {
        let matches = same_component(entry, component);
        removed |= matches;
        !matches
    });
    (entries.collect::<Vec<_>>().join(";"), removed)
}

fn same_component(left: &str, right: &str) -> bool {
    normalize_component(left) == normalize_component(right)
}

fn normalize_component(value: &str) -> String {
    let value = value.trim().trim_matches('"').replace('/', "\\");
    let without_trailing = value.trim_end_matches('\\');
    if without_trailing.len() == 2 && without_trailing.as_bytes()[1] == b':' {
        format!("{}\\", without_trailing.to_ascii_lowercase())
    } else {
        without_trailing.to_ascii_lowercase()
    }
}

fn path_text(path: &Path) -> io::Result<String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "path is not valid Unicode"))
}

fn ignore_missing(result: io::Result<()>) -> io::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn broadcast_environment_change() {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };

    let environment: Vec<u16> = std::ffi::OsStr::new("Environment")
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut result = 0usize;
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            environment.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5_000,
            &mut result,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{add_path_component, remove_path_component};

    #[test]
    fn path_editing_is_exact_idempotent_and_preserves_other_entries() {
        let original = r"C:\Tools;C:\Smart;C:\Smart Explorer Extra;%USERPROFILE%\bin";
        let component = r"C:\Smart Explorer";
        let (added, changed) = add_path_component(original, component);
        assert!(changed);
        assert!(added.ends_with(component));
        let (same, changed) = add_path_component(&added, r"c:/smart explorer\");
        assert!(!changed);
        assert_eq!(same, added);
        let (removed, changed) = remove_path_component(&same, component);
        assert!(changed);
        assert_eq!(removed, original);
    }
}
