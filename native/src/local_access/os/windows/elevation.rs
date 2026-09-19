use super::{broker, image_lock::LockedImage, pipe::Pipe, privilege::BackupRead};
use crate::local_access::protocol::{self, quote, ReadReply, HELPER_FLAG, PIPE_PREFIX};
use std::{
    ffi::OsString,
    io,
    mem::{size_of, zeroed},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr::{null, null_mut},
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_CANCELLED},
    Storage::FileSystem::GetDriveTypeW,
    System::{
        Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE},
        Threading::GetProcessId,
    },
    UI::{
        Shell::{
            ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS,
            SHELLEXECUTEINFOW,
        },
        WindowsAndMessaging::SW_HIDE,
    },
};

fn local_drive(root: &str) -> bool {
    if protocol::validate_root(root).is_err() {
        return false;
    }
    let drive: Vec<_> = format!("{}:\\\0", &root[..1]).encode_utf16().collect();
    matches!(unsafe { GetDriveTypeW(drive.as_ptr()) }, 2 | 3)
}

pub(crate) fn can_request_access(root: &str) -> bool {
    local_drive(root) && !broker::granted(root) && BackupRead::enable().is_err()
}

pub(crate) fn run_helper_if_requested(args: &[OsString]) -> Option<Result<(), String>> {
    protocol::parse(args).map(|request| request.and_then(broker::serve))
}

struct Com;
impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

pub(crate) fn request_access(root: &str) -> Result<bool, String> {
    protocol::validate_root(root)?;
    if !local_drive(root) {
        return Err("Lesezugriff benötigt ein lokales Laufwerk".into());
    }
    if broker::granted(root) || BackupRead::enable().is_ok() {
        return Ok(true);
    }
    let initialized = unsafe {
        CoInitializeEx(
            null(),
            (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
        )
    };
    if initialized < 0 {
        return Err(format!("UAC-Initialisierung: {initialized:#x}"));
    }
    let _com = Com;
    let image = LockedImage::current().map_err(|error| format!("Programm absichern: {error}"))?;
    let mut random = [0u8; 16];
    getrandom::getrandom(&mut random).map_err(|error| error.to_string())?;
    let nonce: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let pipe_name = format!("{PIPE_PREFIX}{nonce}");
    let pipe = Pipe::server(&pipe_name).map_err(|error| error.to_string())?;
    let arguments = [
        HELPER_FLAG.to_string(),
        pipe_name,
        std::process::id().to_string(),
        image.hash.clone(),
        root.to_string(),
    ]
    .iter()
    .map(|argument| quote(argument))
    .collect::<Vec<_>>()
    .join(" ");
    let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let application = wide(&image.path.to_string_lossy());
    let arguments = wide(&arguments);
    let verb = wide("runas");
    let mut execute: SHELLEXECUTEINFOW = unsafe { zeroed() };
    execute.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    execute.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
    execute.hwnd = null_mut();
    execute.lpFile = application.as_ptr();
    execute.lpVerb = verb.as_ptr();
    execute.lpParameters = arguments.as_ptr();
    execute.nShow = SW_HIDE;
    if unsafe { ShellExecuteExW(&mut execute) } == 0 {
        let error = unsafe { GetLastError() };
        if error == ERROR_CANCELLED {
            return Ok(false);
        }
        return Err(format!(
            "Lesezugriff konnte nicht angefordert werden: {}",
            io::Error::from_raw_os_error(error as i32)
        ));
    }
    if execute.hProcess.is_null() {
        return Err("Windows lieferte keinen Lesehelfer-Prozess".into());
    }
    let child = unsafe { OwnedHandle::from_raw_handle(execute.hProcess) };
    pipe.accept(child.as_raw_handle())
        .map_err(|error| error.to_string())?;
    let expected = unsafe { GetProcessId(child.as_raw_handle()) };
    if expected == 0 || pipe.peer_id(true).map_err(|error| error.to_string())? != expected {
        return Err("Unerwarteter Prozess an der Lesehelfer-Verbindung".into());
    }
    let ready: ReadReply = pipe
        .receive(child.as_raw_handle(), Duration::from_secs(30))
        .map_err(|error| format!("Lesehelfer nicht bereit: {error}"))?;
    if let Some(error) = ready.error {
        return Err(io::Error::from_raw_os_error(error).to_string());
    }
    if ready.handle != 0 {
        return Err("Ungültige Lesehelfer-Bestätigung".into());
    }
    broker::install(root.to_string(), pipe, child).map_err(|error| error.to_string())?;
    Ok(true)
}
