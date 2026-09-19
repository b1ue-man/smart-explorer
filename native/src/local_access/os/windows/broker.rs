use super::{image_lock::LockedImage, pipe::Pipe, read::open_scoped};
use crate::local_access::protocol::{self, ReadKind, ReadReply, ReadRequest, Startup};
use std::{
    fs::File,
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::Path,
    ptr::null_mut,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE, WAIT_TIMEOUT},
    System::Threading::{
        GetCurrentProcess, OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject,
        PROCESS_DUP_HANDLE, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    },
};

pub(super) struct Client {
    root: String,
    channel: Mutex<Pipe>,
    child: OwnedHandle,
    healthy: AtomicBool,
}

fn clients() -> &'static Mutex<Vec<Arc<Client>>> {
    static CLIENTS: OnceLock<Mutex<Vec<Arc<Client>>>> = OnceLock::new();
    CLIENTS.get_or_init(Mutex::default)
}

#[cfg(test)]
pub(super) fn remove_test_grant(root: &str) {
    clients()
        .lock()
        .unwrap()
        .retain(|client| client.root != root);
}

impl Client {
    fn alive(&self) -> bool {
        self.healthy.load(Ordering::Acquire)
            && unsafe { WaitForSingleObject(self.child.as_raw_handle(), 0) == WAIT_TIMEOUT }
    }

    fn read(&self, path: String, kind: ReadKind) -> io::Result<File> {
        let channel = self
            .channel
            .lock()
            .map_err(|_| io::Error::other("Lesehelfer-Verbindung unterbrochen"))?;
        if !self.alive() {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        let pipe = &*channel;
        let peer = self.child.as_raw_handle();
        pipe.send(&ReadRequest { path, kind }, peer)
            .inspect_err(|_| self.healthy.store(false, Ordering::Release))?;
        let reply: ReadReply = pipe
            .receive(peer, Duration::from_secs(30))
            .inspect_err(|_| self.healthy.store(false, Ordering::Release))?;
        if let Some(error) = reply.error {
            return Err(io::Error::from_raw_os_error(error));
        }
        let handle =
            usize::try_from(reply.handle).map_err(|_| io::Error::other("Ungültiger Lesehandle"))?;
        if handle == 0 || handle == usize::MAX {
            return Err(io::Error::other("Ungültiger Lesehandle"));
        }
        // The authenticated helper duplicated exactly this read-only handle
        // into our process; ownership passes to File on successful reception.
        Ok(unsafe { File::from_raw_handle(handle as HANDLE) })
    }
}

pub(super) fn install(root: String, pipe: Pipe, child: OwnedHandle) -> io::Result<()> {
    let mut active = clients()
        .lock()
        .map_err(|_| io::Error::other("Lesezugriffe nicht verfügbar"))?;
    active.retain(|client| client.alive() && !protocol::contains(&root, &client.root));
    if active.len() >= 16 {
        return Err(io::Error::other(
            "Zu viele getrennte Lesezugriffe; nicht mehr benötigte Explorer-Fenster schließen",
        ));
    }
    active.push(Arc::new(Client {
        root,
        channel: Mutex::new(pipe),
        child,
        healthy: AtomicBool::new(true),
    }));
    Ok(())
}

pub(super) fn granted(path: &str) -> bool {
    clients().lock().is_ok_and(|active| {
        active
            .iter()
            .any(|client| protocol::contains(&client.root, path) && client.alive())
    })
}

pub(super) fn open_granted(path: &Path, kind: ReadKind) -> Option<io::Result<File>> {
    let path = super::display_path(path).replace('\\', "/");
    let client = clients()
        .lock()
        .ok()?
        .iter()
        .filter(|client| client.alive() && protocol::contains(&client.root, &path))
        .max_by_key(|client| client.root.len())
        .cloned()?;
    Some(client.read(path, kind))
}

pub(super) fn serve(request: Startup) -> Result<(), String> {
    let image = LockedImage::current().map_err(|error| error.to_string())?;
    if image.hash != request.image_sha256 {
        return Err("Programm-Prüfsumme stimmt nicht überein".into());
    }
    let parent = unsafe {
        OpenProcess(
            PROCESS_DUP_HANDLE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            request.parent,
        )
    };
    if parent.is_null() {
        return Err(io::Error::last_os_error().to_string());
    }
    let parent = unsafe { OwnedHandle::from_raw_handle(parent) };
    verify_parent_image(parent.as_raw_handle(), &image)?;
    let pipe = Pipe::client(&request.pipe).map_err(|error| error.to_string())?;
    if pipe.peer_id(false).map_err(|error| error.to_string())? != request.parent {
        return Err("Leseanfrage stammt nicht vom erwarteten Explorer-Prozess".into());
    }
    // Validate the requested root and backup capability before acknowledging
    // consent. No data and no privileged token is transferred during admission.
    let result = open_scoped(&request.root, &request.root, ReadKind::Directory);
    let reply = ReadReply {
        handle: 0,
        error: result.as_ref().err().map(error_code),
    };
    pipe.send(&reply, parent.as_raw_handle())
        .map_err(|error| error.to_string())?;
    result.map_err(|error| error.to_string())?;
    loop {
        let operation: ReadRequest =
            match pipe.receive(parent.as_raw_handle(), Duration::from_secs(60)) {
                Ok(operation) => operation,
                Err(error) if error.kind() == io::ErrorKind::TimedOut => continue,
                Err(_) => return Ok(()),
            };
        let result = open_scoped(&request.root, &operation.path, operation.kind)
            .and_then(|file| duplicate_file(&file, parent.as_raw_handle()));
        let reply = match result {
            Ok(handle) => ReadReply {
                handle,
                error: None,
            },
            Err(error) => ReadReply {
                handle: 0,
                error: Some(error_code(&error)),
            },
        };
        if let Err(error) = pipe.send(&reply, parent.as_raw_handle()) {
            if reply.handle != 0 {
                // Message writes are atomic. A failed send never handed the
                // duplicate to the caller, so close it in the target process.
                unsafe {
                    DuplicateHandle(
                        parent.as_raw_handle(),
                        reply.handle as usize as HANDLE,
                        null_mut(),
                        null_mut(),
                        0,
                        0,
                        windows_sys::Win32::Foundation::DUPLICATE_CLOSE_SOURCE,
                    );
                }
            }
            return Err(error.to_string());
        }
    }
}

fn error_code(error: &io::Error) -> i32 {
    error
        .raw_os_error()
        .unwrap_or(windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED as i32)
}

fn duplicate_file(file: &File, parent: HANDLE) -> io::Result<u64> {
    let mut handle = null_mut();
    if unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            file.as_raw_handle(),
            parent,
            &mut handle,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(handle as usize as u64)
}

fn verify_parent_image(parent: HANDLE, image: &LockedImage) -> Result<(), String> {
    let mut buffer = vec![0u16; 32768];
    let mut size = buffer.len() as u32;
    if unsafe { QueryFullProcessImageNameW(parent, 0, buffer.as_mut_ptr(), &mut size) } == 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    let path = String::from_utf16(&buffer[..size as usize]).map_err(|error| error.to_string())?;
    if !path.eq_ignore_ascii_case(&image.path.to_string_lossy()) {
        return Err("Elternprozess verwendet ein anderes Programm".into());
    }
    Ok(())
}
