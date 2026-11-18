//! The elevated step of the virtual display driver (plan 2026-11-17 P5):
//! run by the installer or after an update with one UAC confirmation, never
//! during use. Its directory `%ProgramData%\SmartExplorer-VirtualDisplay`
//! belongs to the administrators (SYSTEM and Administrators full control,
//! LOCAL SERVICE, under which the driver's host process runs, and Users
//! read); a directory someone else owns is removed and made anew. It holds
//! the signed package (`driver\`), the driver's settings
//! (`vdd_settings.xml`, found by the driver through `VDDPATH`) and the state
//! (`state.json`). What the step does follows `windows_driver::driver_step`:
//! nothing for the installed driver, the signed package again for a broken
//! device or a new mode list, a new signature only for other driver files.
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{LocalFree, ERROR_SUCCESS};
use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
use windows_sys::Win32::Security::{
    IsWellKnownSid, WinBuiltinAdministratorsSid, WinLocalSystemSid, OWNER_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::UI::Shell::{FOLDERID_ProgramData, SHGetKnownFolderPath};

use super::pins::{fetch_driver, sha256_hex, Pins};
use super::{device, sign};
use crate::screen::core::virtual_display::windows_driver::{
    driver_step, settings_xml, DriverFacts, DriverStep, InstalledState, DRIVER_REGISTRY_KEY,
    DRIVER_REGISTRY_PATH_VALUE, HARDWARE_ID, MODE_SIZES, REFRESH_RATES,
};

const DIRECTORY: &str = "SmartExplorer-VirtualDisplay";
const SETTINGS: &str = "vdd_settings.xml";
const STATE: &str = "state.json";
const PACKAGE: &str = "driver";

/// `%ProgramData%` as the shell knows it (not from the environment, which
/// the caller of an elevation chooses).
fn program_data() -> io::Result<PathBuf> {
    let mut path: windows_sys::core::PWSTR = null_mut();
    // SAFETY: the shell allocates `path`; it is freed below.
    let result = unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramData, 0, null_mut(), &mut path) };
    if result < 0 || path.is_null() {
        return Err(io::Error::other(format!(
            "ProgramData: HRESULT {result:#x}"
        )));
    }
    let mut length = 0usize;
    // SAFETY: the shell returns a NUL-terminated string.
    while unsafe { *path.add(length) } != 0 {
        length += 1;
    }
    // SAFETY: `length` units were just read.
    let text = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(path, length) });
    // SAFETY: allocated by the shell with the COM allocator.
    unsafe { CoTaskMemFree(path as *const core::ffi::c_void) };
    Ok(PathBuf::from(text))
}

/// se's directory of the driver.
pub fn directory() -> io::Result<PathBuf> {
    Ok(program_data()?.join(DIRECTORY))
}

/// The state the last elevated step left.
pub fn read_state(dir: &Path) -> Option<InstalledState> {
    serde_json::from_slice(&fs::read(dir.join(STATE)).ok()?).ok()
}

fn file_matches(path: &Path, size: u64, sha256: &str) -> bool {
    fs::read(path).is_ok_and(|bytes| bytes.len() as u64 == size && sha256_hex(&bytes) == sha256)
}

/// The facts of `driver_step` on this device (no rights needed).
pub fn facts(dir: &Path, pins: &Pins) -> DriverFacts {
    let package = dir.join(PACKAGE);
    let package_ok = package.join(pins.files[0].name).is_file()
        && pins.files[1..]
            .iter()
            .all(|pin| file_matches(&package.join(pin.name), pin.size, pin.sha256));
    let settings = settings_xml(&MODE_SIZES, &REFRESH_RATES);
    DriverFacts {
        device_ok: device::working(),
        package_ok,
        settings_ok: fs::read(dir.join(SETTINGS)).is_ok_and(|bytes| bytes == settings.as_bytes()),
    }
}

/// What the step must do now.
pub fn step(pins: &Pins) -> DriverStep {
    match directory() {
        Ok(dir) => driver_step(
            read_state(&dir).as_ref(),
            &pins.driver_hash(),
            facts(&dir, pins),
        ),
        Err(_) => DriverStep::SignAndInstall,
    }
}

/// Whether `path` is owned by SYSTEM or the administrators.
fn owned_by_administrators(path: &Path) -> bool {
    let name: Vec<u16> = path
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut owner: PSID = null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: the name is NUL-terminated; the descriptor is freed below.
    let result = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        )
    };
    if result != ERROR_SUCCESS {
        return false;
    }
    // SAFETY: `owner` points into `descriptor`, alive until LocalFree.
    let trusted = unsafe {
        IsWellKnownSid(owner, WinBuiltinAdministratorsSid) != 0
            || IsWellKnownSid(owner, WinLocalSystemSid) != 0
    };
    // SAFETY: allocated by GetNamedSecurityInfoW.
    unsafe { LocalFree(descriptor) };
    trusted
}

fn system32(program: &str) -> io::Result<PathBuf> {
    Ok(sign::system_directory()
        .map_err(io::Error::other)?
        .join(program))
}

/// Owner Administrators, no inherited entries, SYSTEM and Administrators
/// full control, LOCAL SERVICE and Users read.
fn protect(dir: &Path) -> io::Result<()> {
    let icacls = system32("icacls.exe")?;
    for args in [
        vec!["/setowner", "*S-1-5-32-544", "/T", "/C", "/Q"],
        vec![
            "/inheritance:r",
            "/grant:r",
            "*S-1-5-18:(OI)(CI)F",
            "*S-1-5-32-544:(OI)(CI)F",
            "*S-1-5-19:(OI)(CI)RX",
            "*S-1-5-32-545:(OI)(CI)RX",
            "/T",
            "/C",
            "/Q",
        ],
    ] {
        let mut command = Command::new(&icacls);
        command.arg(dir).args(&args);
        std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
        let status = command.status()?;
        if !status.success() {
            return Err(io::Error::other(format!("icacls {args:?}: {status}")));
        }
    }
    Ok(())
}

/// The directory, made anew unless it is a plain directory of the
/// administrators.
fn secured_directory() -> io::Result<PathBuf> {
    let dir = directory()?;
    let trusted = fs::symlink_metadata(&dir)
        .is_ok_and(|meta| meta.is_dir() && !meta.file_type().is_symlink())
        && owned_by_administrators(&dir);
    if !trusted {
        match fs::symlink_metadata(&dir) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {
                fs::remove_dir_all(&dir)?
            }
            Ok(_) => fs::remove_file(&dir).or_else(|_| fs::remove_dir(&dir))?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::create_dir(&dir)?;
    }
    protect(&dir)?;
    Ok(dir)
}

/// Writes `bytes` as a new file (an old entry of that name goes first).
fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path)?,
        Ok(_) => fs::remove_file(path)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn write_settings(dir: &Path) -> io::Result<()> {
    write_new(
        &dir.join(SETTINGS),
        settings_xml(&MODE_SIZES, &REFRESH_RATES).as_bytes(),
    )?;
    let (key, _) = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
        .create_subkey(DRIVER_REGISTRY_KEY)?;
    key.set_value(DRIVER_REGISTRY_PATH_VALUE, &dir.as_os_str())
}

/// Whether the driver store package `oem` is the driver's (before se removes it).
fn is_driver_package(oem: &str) -> bool {
    let Ok(system) = sign::system_directory() else {
        return false;
    };
    let Some(path) = system.parent().map(|windows| windows.join("INF").join(oem)) else {
        return false;
    };
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    let text = if bytes.starts_with(&[0xff, 0xfe]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    text.to_ascii_lowercase()
        .contains(&HARDWARE_ID.to_ascii_lowercase())
}

/// The elevated step; a line for the log on success.
pub fn run(pins: &Pins) -> Result<String, String> {
    fn fail(what: &str, error: impl std::fmt::Display) -> String {
        format!("{what}: {error}")
    }
    let step = step(pins);
    if step == DriverStep::Nothing {
        return Ok("virtueller Bildschirm: Treiber unverändert".into());
    }
    let mut state = directory()
        .ok()
        .and_then(|dir| read_state(&dir))
        .unwrap_or_default();
    let dir = secured_directory().map_err(|e| fail("Treiberordner", &e))?;
    let package = dir.join(PACKAGE);
    if step == DriverStep::SignAndInstall {
        let files = fetch_driver(pins).map_err(|e| fail("Treiber laden", &e))?;
        if package.exists() {
            fs::remove_dir_all(&package).map_err(|e| fail("alter Treiberordner", &e))?;
        }
        fs::create_dir(&package).map_err(|e| fail("Treiberordner", &e))?;
        for (name, bytes) in &files {
            write_new(&package.join(name), bytes).map_err(|e| fail(name, &e))?;
        }
        let thumbprint = sign::sign_catalog(&package.join(pins.files[0].name))?;
        state = InstalledState {
            driver_hash: pins.driver_hash(),
            release: pins.release.to_string(),
            certificate: thumbprint,
            oem_inf: state.oem_inf,
        };
    }
    write_settings(&dir).map_err(|e| fail("Treibereinstellungen", &e))?;
    let oem = device::install(&package.join(pins.inf().name))?;
    if let Err(error) = sign::forget_other_certificates(&state.certificate) {
        eprintln!("virtueller Bildschirm: alte Zertifikate: {error}");
    }
    if !state.oem_inf.is_empty()
        && !state.oem_inf.eq_ignore_ascii_case(&oem)
        && is_driver_package(&state.oem_inf)
    {
        if let Err(error) = device::remove_package(&state.oem_inf) {
            eprintln!("virtueller Bildschirm: altes Treiberpaket: {error}");
        }
    }
    state.oem_inf = oem;
    let json = serde_json::to_vec_pretty(&state).map_err(|e| fail("Zustand", &e))?;
    write_new(&dir.join(STATE), &json).map_err(|e| fail("Zustand", &e))?;
    Ok(format!(
        "virtueller Bildschirm: {} ({:?}, Paket {})",
        pins.release, step, state.oem_inf
    ))
}
