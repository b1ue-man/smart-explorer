//! The watch thread of Windows (RV1, V4): `ReadDirectoryChangesW` on each
//! root (whole subtree, names, sizes, write and creation times) with overlapped
//! I/O on one completion port. The change buffer is large on local volumes
//! and 64 KiB on network paths (the SMB limit); a completion with zero bytes
//! or `ERROR_NOTIFY_ENUM_DIR` is a lost buffer (`Overflow`). A file system or
//! redirector without change notifications reports `Unsupported`. Local
//! roots release their handle for safe removal (`device`).

use std::collections::HashMap;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED, HANDLE,
    INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY,
    FILE_NOTIFY_CHANGE_CREATION, FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME,
    FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_SECURITY, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Threading::INFINITE;
use windows_sys::Win32::System::IO::{
    CreateIoCompletionPort, GetQueuedCompletionStatus, PostQueuedCompletionStatus, OVERLAPPED,
};

use super::device::{DeviceWatch, HandleCell, DEVICE_BIT};
use super::service::{deliver_owed, emit, overflow_owed, RootSpec};
use super::types::{Coverage, UnavailableReason, WatchEvent, WatchId};

const COMMAND_KEY: usize = 0;
const FILTER: u32 = FILE_NOTIFY_CHANGE_FILE_NAME
    | FILE_NOTIFY_CHANGE_DIR_NAME
    | FILE_NOTIFY_CHANGE_SIZE
    | FILE_NOTIFY_CHANGE_LAST_WRITE
    | FILE_NOTIFY_CHANGE_CREATION
    | FILE_NOTIFY_CHANGE_ATTRIBUTES
    | FILE_NOTIFY_CHANGE_SECURITY;
/// The kernel keeps one buffer of this size per handle between reads: large
/// locally so bursts fit, at most 64 KiB over SMB (larger fails there).
const LOCAL_BUFFER_BYTES: usize = 512 * 1024;
const NETWORK_BUFFER_BYTES: usize = 64 * 1024;
const MISSING_RETRY: Duration = Duration::from_secs(10);
const FIRST_BACKOFF: Duration = Duration::from_secs(60);
const MAX_BACKOFF: Duration = Duration::from_secs(3_600);
const OWED_RETRY: Duration = Duration::from_secs(1);

#[path = "rdcw_io.rs"]
mod io_events;
use io_events::{is_network, issue, reason_of_code, reason_of_io};

enum Command {
    Add(RootSpec),
    Remove(WatchId),
}

pub(crate) struct Backend {
    port: usize,
    commands: Sender<Command>,
}

impl Backend {
    pub(crate) fn start() -> io::Result<Self> {
        // SAFETY: creates a new completion port without a file.
        let port =
            unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, std::ptr::null_mut(), 0, 1) };
        if port.is_null() {
            return Err(io::Error::last_os_error());
        }
        let (commands, receiver) = crossbeam_channel::unbounded();
        let port = port as usize;
        if let Err(error) = std::thread::Builder::new().name("watch-rdcw".into())
            .spawn(move || run(port, receiver)) {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(port as HANDLE); }
            return Err(error);
        }
        Ok(Self { port, commands })
    }

    pub(crate) fn add(&self, spec: RootSpec) {
        self.send(Command::Add(spec));
    }

    pub(crate) fn remove(&self, id: WatchId) {
        self.send(Command::Remove(id));
    }

    fn send(&self, command: Command) {
        if self.commands.send(command).is_ok() {
            // SAFETY: posts a packet without OVERLAPPED to the live port.
            unsafe {
                PostQueuedCompletionStatus(self.port as HANDLE, 0, COMMAND_KEY, std::ptr::null())
            };
        }
    }
}

/// One open handle of a root with its read buffer; it lives until the read
/// in flight has completed, even after the handle was closed.
struct Slot {
    id: WatchId,
    handle: Arc<HandleCell>,
    buffer: Box<[u32]>,
    overlapped: Box<OVERLAPPED>,
    pending: bool,
    closing: bool,
    device: Option<DeviceWatch>,
}

struct Root {
    spec: RootSpec,
    slot: Option<usize>,
    retry_at: Option<Instant>,
    backoff: Duration,
    reported: Option<UnavailableReason>,
}

struct State {
    port: HANDLE,
    roots: HashMap<WatchId, Root>,
    slots: HashMap<usize, Slot>,
    next_key: usize,
}

fn run(port: usize, commands: Receiver<Command>) {
    let mut state = State {
        port: port as HANDLE,
        roots: HashMap::new(),
        slots: HashMap::new(),
        next_key: COMMAND_KEY,
    };
    loop {
        let now = Instant::now();
        state.rearm_due(now);
        let mut timeout = state
            .next_retry()
            .map(|at| at.saturating_duration_since(now));
        if overflow_owed() {
            timeout = Some(timeout.map_or(OWED_RETRY, |wait| wait.min(OWED_RETRY)));
        }
        let millis = timeout.map_or(INFINITE, |wait| {
            u32::try_from(wait.as_millis().max(1)).unwrap_or(INFINITE - 1)
        });
        let mut bytes = 0u32;
        let mut key = 0usize;
        let mut overlapped: *mut OVERLAPPED = std::ptr::null_mut();
        // SAFETY: valid output locations; the port lives for the process.
        let done = unsafe {
            GetQueuedCompletionStatus(state.port, &mut bytes, &mut key, &mut overlapped, millis)
        };
        // SAFETY: reads the calling thread's last error.
        let error = if done != 0 {
            0
        } else {
            unsafe { GetLastError() }
        };
        if done == 0 && overlapped.is_null() {
            if error != WAIT_TIMEOUT {
                std::thread::sleep(OWED_RETRY);
            }
        } else if key == COMMAND_KEY {
            while let Ok(command) = commands.try_recv() {
                match command {
                    Command::Add(spec) => state.add(spec),
                    Command::Remove(id) => state.remove(id),
                }
            }
        } else if key & DEVICE_BIT != 0 {
            state.device_event(key & !DEVICE_BIT);
        } else {
            state.completion(key, error, bytes);
        }
        deliver_owed();
    }
}

impl State {
    fn add(&mut self, spec: RootSpec) {
        let id = spec.id;
        self.roots.insert(
            id,
            Root {
                spec,
                slot: None,
                retry_at: None,
                backoff: FIRST_BACKOFF,
                reported: None,
            },
        );
        self.arm(id);
    }

    fn remove(&mut self, id: WatchId) {
        if let Some(root) = self.roots.remove(&id) {
            if let Some(key) = root.slot {
                self.close_slot(key);
            }
        }
    }

    fn rearm_due(&mut self, now: Instant) {
        let due: Vec<WatchId> = self
            .roots
            .iter()
            .filter(|(_, root)| root.retry_at.is_some_and(|at| at <= now))
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            self.arm(id);
        }
    }

    fn next_retry(&self) -> Option<Instant> {
        self.roots.values().filter_map(|root| root.retry_at).min()
    }

    fn arm(&mut self, id: WatchId) {
        let Some(root) = self.roots.get_mut(&id) else {
            return;
        };
        root.retry_at = None;
        if let Some(key) = root.slot.take() {
            self.close_slot(key);
        }
        if self.roots.get(&id).is_some_and(|root| root.spec.anchor.is_some()) {
            // Read pins are not overlapped capabilities. Never reopen a
            // confined display path, even if another caller bypasses setup.
            return self.fail(id, UnavailableReason::Unsupported);
        }
        let path = match self.roots.get(&id) {
            Some(root) => root.spec.root.clone(),
            None => return,
        };
        match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return self.fail(id, UnavailableReason::Unsupported),
            Err(error) => return self.fail(id, reason_of_io(&error)),
        }
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: NUL-terminated path; the handle is checked below.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_LIST_DIRECTORY,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            // SAFETY: reads the calling thread's last error.
            let code = unsafe { GetLastError() };
            return self.fail(id, reason_of_code(code));
        }
        self.next_key += 1;
        let key = self.next_key;
        let cell = Arc::new(HandleCell::new(handle));
        // SAFETY: associates the fresh handle with the live port.
        if unsafe { CreateIoCompletionPort(handle, self.port, key, 0) }.is_null() {
            let error = io::Error::last_os_error();
            cell.close();
            return self.fail(
                id,
                UnavailableReason::Failed(format!("Abschlussport: {error}")),
            );
        }
        let network = is_network(&path);
        let bytes = if network {
            NETWORK_BUFFER_BYTES
        } else {
            LOCAL_BUFFER_BYTES
        };
        let device = (!network)
            .then(|| DeviceWatch::register(&cell, self.port, key))
            .flatten();
        if !network && device.is_none() {
            cell.close();
            return self.fail(id, UnavailableReason::Failed("Geräteentfernung kann nicht sicher überwacht werden".into()));
        }
        let mut slot = Slot {
            id,
            handle: cell,
            buffer: vec![0u32; bytes / 4].into_boxed_slice(),
            // SAFETY: an all-zero OVERLAPPED is valid.
            overlapped: Box::new(unsafe { std::mem::zeroed() }),
            pending: false,
            closing: false,
            device,
        };
        if let Err(code) = issue(&mut slot) {
            self.slots.insert(key, slot);
            self.close_slot(key);
            let reason = match code {
                ERROR_INVALID_FUNCTION | ERROR_NOT_SUPPORTED | ERROR_INVALID_PARAMETER => {
                    UnavailableReason::Unsupported
                }
                other => reason_of_code(other),
            };
            return self.fail(id, reason);
        }
        self.slots.insert(key, slot);
        if let Some(root) = self.roots.get_mut(&id) {
            root.slot = Some(key);
            root.backoff = FIRST_BACKOFF;
            root.reported = None;
        }
        // NAS/redirector notification support varies. Keep the verification
        // poll even when ReadDirectoryChangesW accepted the network handle.
        emit(id, vec![WatchEvent::Ready(if network { Coverage::LocalOnly } else { Coverage::Complete })]);
    }

    fn fail(&mut self, id: WatchId, reason: UnavailableReason) {
        let Some(root) = self.roots.get_mut(&id) else {
            return;
        };
        let slot = root.slot.take();
        let now = Instant::now();
        root.retry_at = match reason {
            UnavailableReason::Unsupported => None,
            UnavailableReason::RootMissing | UnavailableReason::DeviceRemoved => {
                Some(now + MISSING_RETRY)
            }
            _ => {
                let wait = root.backoff;
                root.backoff = (wait * 2).min(MAX_BACKOFF);
                Some(now + wait)
            }
        };
        let report = root.reported.as_ref() != Some(&reason);
        if report {
            root.reported = Some(reason.clone());
        }
        if let Some(key) = slot {
            self.close_slot(key);
        }
        if report {
            emit(id, vec![WatchEvent::Unavailable(reason)]);
        }
    }

    /// Closes a slot's handle; the slot stays until its read completed.
    fn close_slot(&mut self, key: usize) {
        let Some(slot) = self.slots.get_mut(&key) else {
            return;
        };
        slot.handle.close();
        if let Some(device) = slot.device.take() {
            device.unregister();
        }
        if slot.pending {
            slot.closing = true;
        } else {
            self.slots.remove(&key);
        }
    }
}
