use super::{image_lock::LockedImage, privilege::BackupRead};
use crate::analytics::{
    access::{validate_local_root, ANALYSIS_ADMIN_FLAG},
    AnalysisStartup,
};
use std::{
    ffi::OsStr,
    mem::{size_of, zeroed},
    os::windows::ffi::OsStrExt,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, ERROR_CANCELLED},
    Storage::FileSystem::GetDriveTypeW,
    System::Com::{
        CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    },
    UI::{
        Shell::{
            ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS,
            SHELLEXECUTEINFOW,
        },
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

fn local_drive(root: &str) -> bool {
    if validate_local_root(root).is_err() {
        return false;
    }
    let drive: Vec<u16> = format!("{}:\\\0", &root[..1]).encode_utf16().collect();
    // DRIVE_REMOVABLE / DRIVE_FIXED. Mapped shares need server credentials, not UAC.
    matches!(unsafe { GetDriveTypeW(drive.as_ptr()) }, 2 | 3)
}

pub(in crate::analytics::os) fn can_request_elevation(root: &str) -> bool {
    local_drive(root) && BackupRead::enable().is_err()
}

pub(in crate::analytics::os) fn verify_analysis_startup(
    request: &AnalysisStartup,
) -> Result<(), String> {
    if !local_drive(&request.root) {
        return Err("Analyse-Ziel ist kein lokales Laufwerk".into());
    }
    let image = LockedImage::current().map_err(|error| format!("Programm absichern: {error}"))?;
    if image.hash != request.image_sha256 {
        return Err("Programm-Prüfsumme stimmt nicht überein".into());
    }
    let _backup = BackupRead::enable()
        .map_err(|error| format!("Windows hat das Sicherungsleserecht nicht gewährt: {error}"))?;
    Ok(())
}

fn wide(value: &OsStr) -> Result<Vec<u16>, String> {
    let mut result: Vec<_> = value.encode_wide().collect();
    if result.contains(&0) {
        return Err("Prozessargument enthält NUL".into());
    }
    result.push(0);
    Ok(result)
}

// CRT argument quoting: trailing backslashes must be doubled before a closing
// quote. Validation excludes embedded quotes but not root/trailing separators.
pub(super) fn parameters(root: &str, hash: &str) -> Result<String, String> {
    validate_local_root(root)?;
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Ungültige Programm-Prüfsumme".into());
    }
    let root = root.replace('/', "\\");
    let trailing = root.chars().rev().take_while(|c| *c == '\\').count();
    Ok(format!(
        "{ANALYSIS_ADMIN_FLAG} \"{root}{}\" --image-sha256 {hash}",
        "\\".repeat(trailing)
    ))
}

struct Com;
impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

pub(in crate::analytics::os) fn launch_elevated_analysis(root: &str) -> Result<bool, String> {
    if !can_request_elevation(root) {
        return Err("Erneute Rechteanfrage ist für dieses Ziel nicht verfügbar".into());
    }
    // Caller owns a fresh background thread; never run this on the GUI thread.
    let initialized = unsafe {
        CoInitializeEx(
            null(),
            (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
        )
    };
    if initialized < 0 {
        return Err(format!(
            "UAC-COM-Initialisierung: 0x{:08x}",
            initialized as u32
        ));
    }
    let _com = Com;
    let image = LockedImage::current().map_err(|error| format!("Programm absichern: {error}"))?;
    let application = wide(image.path.as_os_str())?;
    let verb = wide(OsStr::new("runas"))?;
    let arguments = wide(OsStr::new(&parameters(root, &image.hash)?))?;
    let directory = wide(
        image
            .path
            .parent()
            .ok_or("Programm ohne Elternpfad")?
            .as_os_str(),
    )?;
    let mut execute: SHELLEXECUTEINFOW = unsafe { zeroed() };
    execute.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    execute.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
    execute.hwnd = null_mut();
    execute.lpVerb = verb.as_ptr();
    execute.lpFile = application.as_ptr();
    execute.lpParameters = arguments.as_ptr();
    execute.lpDirectory = directory.as_ptr();
    execute.nShow = SW_SHOWNORMAL;
    let success = unsafe { ShellExecuteExW(&mut execute) };
    let error = unsafe { GetLastError() };
    if !execute.hProcess.is_null() {
        unsafe {
            CloseHandle(execute.hProcess);
        }
    }
    launch_result(success != 0, error)
}

fn launch_result(success: bool, error: u32) -> Result<bool, String> {
    if !success {
        if error == ERROR_CANCELLED {
            return Ok(false);
        }
        return Err(format!(
            "Administrator-Analyse konnte nicht starten: Win32 {error}"
        ));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn analytics_access_task_uac_arguments_and_canceled_consent() {
        let hash = "a".repeat(64);
        for root in ["C:/", "C:\\", "C:/Vault with spaces/", "C:/日本語/"] {
            let command = wide(OsStr::new(&format!(
                "app.exe {}",
                parameters(root, &hash).unwrap()
            )))
            .unwrap();
            let mut count = 0;
            let argv = unsafe {
                windows_sys::Win32::UI::Shell::CommandLineToArgvW(command.as_ptr(), &mut count)
            };
            assert!(!argv.is_null());
            let args: Vec<String> = (0..count as usize)
                .map(|index| unsafe {
                    let pointer = *argv.add(index);
                    let mut len = 0;
                    while *pointer.add(len) != 0 {
                        len += 1;
                    }
                    String::from_utf16(std::slice::from_raw_parts(pointer, len)).unwrap()
                })
                .collect();
            unsafe {
                windows_sys::Win32::Foundation::LocalFree(argv.cast());
            }
            assert_eq!(
                args,
                [
                    "app.exe",
                    ANALYSIS_ADMIN_FLAG,
                    &root.replace('/', "\\"),
                    "--image-sha256",
                    &hash
                ]
            );
        }
        assert_eq!(launch_result(false, ERROR_CANCELLED).unwrap(), false);
        assert!(launch_result(false, 5).is_err());
        assert_eq!(launch_result(true, 5).unwrap(), true); // last error is undefined on success
        assert!(!can_request_elevation("\\\\server\\share"));
    }
    #[test]
    #[ignore = "current image and backup-token checks: remote Windows task only"]
    fn analytics_access_task_startup_binds_current_image_hash() {
        assert_eq!(
            std::env::var("SMART_EXPLORER_ANALYTICS_TASK").as_deref(),
            Ok("1")
        );
        let fixture = tempfile::tempdir().unwrap();
        let image = LockedImage::current().unwrap();
        let mut request = AnalysisStartup {
            root: fixture.path().to_string_lossy().into_owned(),
            image_sha256: image.hash.clone(),
        };
        verify_analysis_startup(&request).unwrap();
        request.image_sha256 = "0".repeat(64);
        assert!(verify_analysis_startup(&request)
            .unwrap_err()
            .contains("Prüfsumme"));
    }
}
