use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    mem::{size_of, zeroed},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
    ptr::{null, null_mut},
};

use sha2::{Digest, Sha256};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, RPC_E_CHANGED_MODE, WAIT_FAILED, WAIT_OBJECT_0},
    Security::WinTrust::{
        WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_FILE_INFO,
        WTD_CHOICE_FILE, WTD_DISABLE_MD2_MD4, WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT,
        WTD_REVOKE_WHOLECHAIN, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY,
        WTD_UICONTEXT_INSTALL, WTD_UI_NONE,
    },
    Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_FLAG_SEQUENTIAL_SCAN, FILE_SHARE_READ,
    },
    System::{
        Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE},
        SystemInformation::GetSystemDirectoryW,
        Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE},
    },
    UI::{
        Shell::{
            ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS,
            SHELLEXECUTEINFOW,
        },
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

use super::runtime_install_download::PinnedMsi;

pub(super) struct LockedMsi {
    file: File,
    path: PathBuf,
    _parent_chain: Vec<File>,
}

impl LockedMsi {
    pub(super) fn open(path: &Path, pinned: &PinnedMsi) -> Result<Self, String> {
        if !path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::CurDir | std::path::Component::ParentDir
                )
            })
        {
            return Err("Dokany-MSI-Pfad ist nicht absolut und normalisiert".into());
        }
        // `msiexec` receives a pathname rather than our already verified file
        // handle. Keep every pathname ancestor open without share-delete so a
        // same-user process cannot replace the verified path before elevation.
        let parent_chain = lock_parent_chain(path)?;
        let mut file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_SEQUENTIAL_SCAN)
            .open(path)
            .map_err(|error| format!("Dokany-MSI exklusiv zum Lesen oeffnen: {error}"))?;
        let metadata = file
            .metadata()
            .map_err(|error| format!("Dokany-MSI-Metadaten lesen: {error}"))?;
        if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("Dokany-MSI ist keine direkte regulaere Datei".into());
        }
        if metadata.len() != pinned.size {
            return Err(format!(
                "Dokany-MSI hat die falsche Groesse (erwartet {}, gefunden {})",
                pinned.size,
                metadata.len()
            ));
        }
        let actual = sha256_locked(&mut file)?;
        if actual != pinned.sha256 {
            return Err(
                "Dokany-MSI stimmt nicht mit der fest eingebetteten SHA-256 ueberein".into(),
            );
        }
        Ok(Self {
            file,
            path: path.to_path_buf(),
            _parent_chain: parent_chain,
        })
    }

    pub(super) fn verify_authenticode(&self) -> Result<(), String> {
        let path = wide_nul(self.path.as_os_str())?;
        let mut file_info: WINTRUST_FILE_INFO = unsafe { zeroed() };
        file_info.cbStruct = size_of::<WINTRUST_FILE_INFO>() as u32;
        file_info.pcwszFilePath = path.as_ptr();
        file_info.hFile = self.file.as_raw_handle().cast();

        let mut trust: WINTRUST_DATA = unsafe { zeroed() };
        trust.cbStruct = size_of::<WINTRUST_DATA>() as u32;
        trust.dwUIChoice = WTD_UI_NONE;
        trust.fdwRevocationChecks = WTD_REVOKE_WHOLECHAIN;
        trust.dwUnionChoice = WTD_CHOICE_FILE;
        trust.Anonymous.pFile = &mut file_info;
        trust.dwStateAction = WTD_STATEACTION_VERIFY;
        trust.dwProvFlags = WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT | WTD_DISABLE_MD2_MD4;
        trust.dwUIContext = WTD_UICONTEXT_INSTALL;

        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        let status = unsafe {
            WinVerifyTrust(
                null_mut(),
                &mut action,
                (&mut trust as *mut WINTRUST_DATA).cast(),
            )
        };
        trust.dwStateAction = WTD_STATEACTION_CLOSE;
        let _ = unsafe {
            WinVerifyTrust(
                null_mut(),
                &mut action,
                (&mut trust as *mut WINTRUST_DATA).cast(),
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(format!(
                "Dokany-MSI besitzt keine gueltige vertrauenswuerdige Authenticode-Signatur (0x{:08X})",
                status as u32
            ))
        }
    }

    pub(super) fn run_elevated(&self) -> Result<u32, ElevatedLaunchError> {
        let _com = ComApartment::initialize().map_err(ElevatedLaunchError::Other)?;
        let msiexec = system_directory()
            .map_err(ElevatedLaunchError::Other)?
            .join("msiexec.exe");
        let verb = wide_nul(std::ffi::OsStr::new("runas")).map_err(ElevatedLaunchError::Other)?;
        let application = wide_nul(msiexec.as_os_str()).map_err(ElevatedLaunchError::Other)?;
        let parameters = msiexec_parameters(&self.path)
            .and_then(|parameters| wide_nul(&parameters))
            .map_err(ElevatedLaunchError::Other)?;

        let mut execute: SHELLEXECUTEINFOW = unsafe { zeroed() };
        execute.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        execute.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
        execute.hwnd = null_mut();
        execute.lpVerb = verb.as_ptr();
        execute.lpFile = application.as_ptr();
        execute.lpParameters = parameters.as_ptr();
        execute.lpDirectory = null();
        execute.nShow = SW_SHOWNORMAL;

        if unsafe { ShellExecuteExW(&mut execute) } == 0 {
            let code = unsafe { GetLastError() };
            return if code == 1223 {
                Err(ElevatedLaunchError::Cancelled)
            } else {
                Err(ElevatedLaunchError::Other(format!(
                    "Windows-UAC/MSI-Start fehlgeschlagen (Win32 {code})"
                )))
            };
        }
        if execute.hProcess.is_null() {
            return Err(ElevatedLaunchError::Other(
                "Windows lieferte keinen Prozesshandle fuer die Dokany-Installation".into(),
            ));
        }
        let process = OwnedHandle(execute.hProcess);
        match unsafe { WaitForSingleObject(process.0, INFINITE) } {
            WAIT_OBJECT_0 => {}
            WAIT_FAILED => {
                return Err(ElevatedLaunchError::Other(format!(
                    "auf Dokany-Installation warten: Win32 {}",
                    unsafe { GetLastError() }
                )))
            }
            value => {
                return Err(ElevatedLaunchError::Other(format!(
                    "unerwartetes Windows-Warteergebnis fuer Dokany: {value}"
                )))
            }
        }
        let mut exit_code = 0u32;
        if unsafe { GetExitCodeProcess(process.0, &mut exit_code) } == 0 {
            return Err(ElevatedLaunchError::Other(format!(
                "Dokany-MSI-Exitcode lesen: Win32 {}",
                unsafe { GetLastError() }
            )));
        }
        Ok(exit_code)
    }
}

fn lock_parent_chain(path: &Path) -> Result<Vec<File>, String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Dokany-MSI besitzt keinen sicheren Elternordner".to_string())?;
    let mut directories = parent.ancestors().collect::<Vec<_>>();
    directories.reverse();
    let mut locked = Vec::with_capacity(directories.len());
    for directory in directories {
        // Drive and share roots cannot be renamed as ordinary children. Do not
        // impose a share-mode lock on the volume root itself.
        if directory.parent().is_none() {
            continue;
        }
        let handle = OpenOptions::new()
            .access_mode(0)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(directory)
            .map_err(|error| {
                format!(
                    "Dokany-MSI-Pfadkomponente absichern ({}): {error}",
                    directory.display()
                )
            })?;
        let metadata = handle.metadata().map_err(|error| {
            format!(
                "Dokany-MSI-Pfadkomponente pruefen ({}): {error}",
                directory.display()
            )
        })?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(format!(
                "Dokany-MSI-Pfad durchquert keinen direkten regulaeren Ordner: {}",
                directory.display()
            ));
        }
        locked.push(handle);
    }
    Ok(locked)
}

pub(super) enum ElevatedLaunchError {
    Cancelled,
    Other(String),
}

enum ComApartment {
    InitializedHere,
    AlreadyInitialized,
}

impl ComApartment {
    fn initialize() -> Result<Self, String> {
        let result = unsafe {
            CoInitializeEx(
                null(),
                (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
            )
        };
        match result {
            0 | 1 => Ok(Self::InitializedHere),
            RPC_E_CHANGED_MODE => Ok(Self::AlreadyInitialized),
            code => Err(format!(
                "Windows-COM fuer die Administratorabfrage initialisieren: 0x{:08X}",
                code as u32
            )),
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if matches!(self, Self::InitializedHere) {
            unsafe { CoUninitialize() };
        }
    }
}

struct OwnedHandle(windows_sys::Win32::Foundation::HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

fn sha256_locked(file: &mut File) -> Result<String, String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("Dokany-MSI zur Pruefung positionieren: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Dokany-MSI hashen: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("Dokany-MSI nach Pruefung positionieren: {error}"))?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn system_directory() -> Result<PathBuf, String> {
    let mut buffer = vec![0u16; 260];
    loop {
        let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
        if length == 0 {
            return Err(format!(
                "Windows-Systemordner bestimmen: Win32 {}",
                unsafe { GetLastError() }
            ));
        }
        if (length as usize) < buffer.len() {
            buffer.truncate(length as usize);
            return Ok(PathBuf::from(std::ffi::OsString::from_wide(&buffer)));
        }
        buffer.resize(length as usize + 1, 0);
    }
}

fn msiexec_parameters(path: &Path) -> Result<std::ffi::OsString, String> {
    if path
        .as_os_str()
        .encode_wide()
        .any(|unit| unit == b'"' as u16)
    {
        return Err("Dokany-MSI-Pfad enthaelt ein unzulaessiges Anfuehrungszeichen".into());
    }
    let mut parameters = std::ffi::OsString::from("/i \"");
    parameters.push(path.as_os_str());
    parameters.push("\" /passive /norestart ADDLOCAL=DokanDriverFeature INSTALLDEVFILES=0");
    Ok(parameters)
}

fn wide_nul(value: &std::ffi::OsStr) -> Result<Vec<u16>, String> {
    let mut wide = value.encode_wide().collect::<Vec<_>>();
    if wide.contains(&0) {
        return Err("Windows-Prozessargument enthaelt ein eingebettetes NUL-Zeichen".into());
    }
    wide.push(0);
    Ok(wide)
}
