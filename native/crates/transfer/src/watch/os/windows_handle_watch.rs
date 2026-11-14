//! Confined watches of Windows (M14, WIN-SHARE-WATCH): a Share export is
//! watched from its held directory pin, never from its display path. Each
//! watch gets the pin's own overlapped handle (`DirectoryHandle::open_watch`:
//! `ReOpenFile` of the pin, identity-checked against it) and reads it with
//! `ReadDirectoryChangesW` over the whole subtree on one completion port of
//! this thread. A completion with zero bytes or `ERROR_NOTIFY_ENUM_DIR` is a
//! lost buffer (`Overflow`, the consumer rescans). Changes below a link,
//! junction or other redirecting child are not content of the root and are
//! dropped (`under_link`). Removing the watch cancels its read and closes the
//! handle; the read buffer lives until the aborted read has completed.
//!
//! Errors end the watch with `Unavailable(reason)`: a file system or
//! redirector without change notifications is `Unsupported`, which the Share
//! host passes on so that the subscriber polls instead of subscribing again.

use std::collections::HashMap;
use std::io;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_INVALID_FUNCTION,
    ERROR_INVALID_PARAMETER, ERROR_NOTIFY_CLEANUP, ERROR_NOTIFY_ENUM_DIR, ERROR_NOT_SUPPORTED,
    ERROR_OPERATION_ABORTED, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::{
    ReadDirectoryChangesW, FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_CREATION,
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
    FILE_NOTIFY_CHANGE_SECURITY, FILE_NOTIFY_CHANGE_SIZE,
};
use windows_sys::Win32::System::Threading::INFINITE;
use windows_sys::Win32::System::IO::{
    CancelIoEx, CreateIoCompletionPort, GetQueuedCompletionStatus, PostQueuedCompletionStatus,
    OVERLAPPED,
};

// This module is declared inside `service`; the watch modules are one up.
use super::super::notify_records::parse;
use super::super::types::{Change, Coverage, UnavailableReason, WatchEvent, WatchId};
use super::{deliver_owed, emit, overflow_owed};
use crate::local_access::platform::directory_watch::WatchDirectory;
use crate::local_access::DirectoryHandle;

const COMMAND_KEY: usize = 0;
/// The same changes the path-based Windows backend reports.
const FILTER: u32 = FILE_NOTIFY_CHANGE_FILE_NAME
    | FILE_NOTIFY_CHANGE_DIR_NAME
    | FILE_NOTIFY_CHANGE_SIZE
    | FILE_NOTIFY_CHANGE_LAST_WRITE
    | FILE_NOTIFY_CHANGE_CREATION
    | FILE_NOTIFY_CHANGE_ATTRIBUTES
    | FILE_NOTIFY_CHANGE_SECURITY;
/// Large locally so bursts fit; at most 64 KiB over SMB (Microsoft: larger
/// buffers fail with `ERROR_INVALID_PARAMETER` on network directories).
const LOCAL_BUFFER_BYTES: usize = 512 * 1024;
const NETWORK_BUFFER_BYTES: usize = 64 * 1024;
const OWED_RETRY: Duration = Duration::from_secs(1);

enum Command {
    Add(WatchId, DirectoryHandle),
    Remove(WatchId),
}

struct Worker {
    port: usize,
    commands: Sender<Command>,
}

static WORKER: Mutex<Option<Worker>> = Mutex::new(None);

/// Arms a confined watch on the worker thread (started on first use). The
/// watch reports `Ready` or `Unavailable` through the service.
pub(super) fn add(id: WatchId, anchor: DirectoryHandle) -> io::Result<()> {
    let mut worker = WORKER.lock().unwrap_or_else(PoisonError::into_inner);
    if worker.is_none() {
        *worker = Some(start()?);
    }
    if let Some(worker) = worker.as_ref() {
        send(worker, Command::Add(id, anchor));
    }
    Ok(())
}

/// Stops a confined watch; unknown ids are ignored.
pub(super) fn remove(id: WatchId) {
    let worker = WORKER.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(worker) = worker.as_ref() {
        send(worker, Command::Remove(id));
    }
}

fn send(worker: &Worker, command: Command) {
    if worker.commands.send(command).is_ok() {
        // SAFETY: posts a packet without OVERLAPPED to the live port.
        unsafe {
            PostQueuedCompletionStatus(worker.port as HANDLE, 0, COMMAND_KEY, std::ptr::null())
        };
    }
}

fn start() -> io::Result<Worker> {
    // SAFETY: creates a new completion port without a file.
    let port = unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, std::ptr::null_mut(), 0, 1) };
    if port.is_null() {
        return Err(io::Error::last_os_error());
    }
    let (commands, receiver) = crossbeam_channel::unbounded();
    let port = port as usize;
    if let Err(error) = std::thread::Builder::new()
        .name("watch-rdcw-pin".into())
        .spawn(move || run(port, receiver))
    {
        // SAFETY: the port was created above and is not used elsewhere.
        unsafe { CloseHandle(port as HANDLE) };
        return Err(error);
    }
    Ok(Worker { port, commands })
}

/// One armed watch. Buffer and OVERLAPPED stay until the read in flight has
/// completed, even after the handle was closed.
struct Slot {
    id: WatchId,
    anchor: DirectoryHandle,
    directory: Option<WatchDirectory>,
    buffer: Box<[u32]>,
    overlapped: Box<OVERLAPPED>,
    pending: bool,
}

struct State {
    port: HANDLE,
    slots: HashMap<usize, Slot>,
    keys: HashMap<WatchId, usize>,
    next_key: usize,
}

fn run(port: usize, commands: Receiver<Command>) {
    let mut state = State {
        port: port as HANDLE,
        slots: HashMap::new(),
        keys: HashMap::new(),
        next_key: COMMAND_KEY,
    };
    loop {
        let millis = if overflow_owed() {
            u32::try_from(OWED_RETRY.as_millis()).unwrap_or(1_000)
        } else {
            INFINITE
        };
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
                    Command::Add(id, anchor) => state.add(id, anchor),
                    Command::Remove(id) => state.remove(id),
                }
            }
        } else {
            state.completion(key, error, bytes);
        }
        deliver_owed();
    }
}

impl State {
    fn add(&mut self, id: WatchId, anchor: DirectoryHandle) {
        self.remove(id);
        let directory = match anchor.open_watch() {
            Ok(directory) => directory,
            Err(error) => return emit(id, vec![WatchEvent::Unavailable(reason_of_io(&error))]),
        };
        self.next_key += 1;
        let key = self.next_key;
        let handle = directory.raw_handle();
        // SAFETY: associates the fresh handle with the live port.
        if unsafe { CreateIoCompletionPort(handle, self.port, key, 0) }.is_null() {
            let error = io::Error::last_os_error();
            return emit(
                id,
                vec![WatchEvent::Unavailable(UnavailableReason::Failed(format!(
                    "Abschlussport: {error}"
                )))],
            );
        }
        let remote = directory.is_remote();
        let bytes = if remote {
            NETWORK_BUFFER_BYTES
        } else {
            LOCAL_BUFFER_BYTES
        };
        let mut slot = Slot {
            id,
            anchor,
            directory: Some(directory),
            buffer: vec![0u32; bytes / 4].into_boxed_slice(),
            // SAFETY: an all-zero OVERLAPPED is valid.
            overlapped: Box::new(unsafe { std::mem::zeroed() }),
            pending: false,
        };
        if let Err(code) = issue(&mut slot) {
            // Nothing is in flight: the handle closes with the slot.
            return emit(id, vec![WatchEvent::Unavailable(reason_of_code(code))]);
        }
        self.slots.insert(key, slot);
        self.keys.insert(id, key);
        // Redirector notification support varies: keep verification polls.
        emit(
            id,
            vec![WatchEvent::Ready(if remote {
                Coverage::LocalOnly
            } else {
                Coverage::Complete
            })],
        );
    }

    fn remove(&mut self, id: WatchId) {
        if let Some(key) = self.keys.remove(&id) {
            self.close(key);
        }
    }

    /// Cancels the read and closes the handle; a slot with a read in flight
    /// stays until its aborted completion arrives.
    fn close(&mut self, key: usize) {
        let Some(slot) = self.slots.get_mut(&key) else {
            return;
        };
        if let Some(directory) = slot.directory.take() {
            // SAFETY: the handle is open until `directory` is dropped below.
            unsafe { CancelIoEx(directory.raw_handle(), std::ptr::null()) };
            drop(directory);
        }
        if !slot.pending {
            self.slots.remove(&key);
        }
    }

    fn completion(&mut self, key: usize, error: u32, bytes: u32) {
        let Some(slot) = self.slots.get_mut(&key) else {
            return;
        };
        slot.pending = false;
        if slot.directory.is_none() {
            self.slots.remove(&key);
            return;
        }
        let id = slot.id;
        let events = match error {
            0 if bytes == 0 => vec![WatchEvent::Overflow],
            0 => changes(slot, bytes),
            ERROR_NOTIFY_ENUM_DIR => vec![WatchEvent::Overflow],
            ERROR_OPERATION_ABORTED | ERROR_NOTIFY_CLEANUP => {
                self.end(id, key, UnavailableReason::RootMissing);
                return;
            }
            other => {
                self.end(id, key, reason_of_code(other));
                return;
            }
        };
        emit(id, events);
        let Some(slot) = self.slots.get_mut(&key) else {
            return;
        };
        if slot.directory.is_none() {
            return;
        }
        if let Err(code) = issue(slot) {
            self.end(id, key, reason_of_code(code));
        }
    }

    fn end(&mut self, id: WatchId, key: usize, reason: UnavailableReason) {
        if self.keys.get(&id) == Some(&key) {
            self.keys.remove(&id);
        }
        self.close(key);
        emit(id, vec![WatchEvent::Unavailable(reason)]);
    }
}

/// The records of one completed read, without changes below a link child.
fn changes(slot: &Slot, bytes: u32) -> Vec<WatchEvent> {
    let length = usize::try_from(bytes)
        .unwrap_or(0)
        .min(slot.buffer.len() * 4);
    // SAFETY: the kernel wrote `length` bytes into the buffer.
    let raw = unsafe { std::slice::from_raw_parts(slot.buffer.as_ptr().cast::<u8>(), length) };
    let mut verdicts: HashMap<String, bool> = HashMap::new();
    parse(raw)
        .into_iter()
        .filter(|record| {
            let parent = record.rel.rsplit_once('/').map_or("", |(parent, _)| parent);
            if parent.is_empty() {
                return true;
            }
            !*verdicts
                .entry(parent.to_owned())
                .or_insert_with(|| slot.anchor.under_link(&record.rel))
        })
        .map(|record| {
            WatchEvent::Change(Change {
                rel: record.rel,
                kind: record.kind,
                is_dir: None,
            })
        })
        .collect()
}

/// Starts the next read; `Err(code)` when it could not be started.
fn issue(slot: &mut Slot) -> Result<(), u32> {
    let Some(directory) = slot.directory.as_ref() else {
        return Err(ERROR_OPERATION_ABORTED);
    };
    let length = u32::try_from(slot.buffer.len() * 4).unwrap_or(u32::MAX);
    // SAFETY: an all-zero OVERLAPPED is valid (no read is in flight here).
    *slot.overlapped = unsafe { std::mem::zeroed() };
    // SAFETY: buffer (DWORD-aligned) and OVERLAPPED live in the slot until
    // the read completes; the handle is open while `directory` is held.
    let done = unsafe {
        ReadDirectoryChangesW(
            directory.raw_handle(),
            slot.buffer.as_mut_ptr().cast(),
            length,
            1,
            FILTER,
            std::ptr::null_mut(),
            &mut *slot.overlapped,
            None,
        )
    };
    if done == 0 {
        // SAFETY: reads the calling thread's last error right away.
        return Err(unsafe { GetLastError() });
    }
    slot.pending = true;
    Ok(())
}

pub(super) fn reason_of_code(code: u32) -> UnavailableReason {
    match code {
        ERROR_INVALID_FUNCTION | ERROR_INVALID_PARAMETER | ERROR_NOT_SUPPORTED => {
            UnavailableReason::Unsupported
        }
        ERROR_ACCESS_DENIED => UnavailableReason::AccessDenied,
        other => UnavailableReason::Failed(
            io::Error::from_raw_os_error(i32::try_from(other).unwrap_or(i32::MAX)).to_string(),
        ),
    }
}

fn reason_of_io(error: &io::Error) -> UnavailableReason {
    match error.kind() {
        io::ErrorKind::NotFound => UnavailableReason::RootMissing,
        io::ErrorKind::PermissionDenied => UnavailableReason::AccessDenied,
        io::ErrorKind::Unsupported => UnavailableReason::Unsupported,
        _ => match error
            .raw_os_error()
            .and_then(|code| u32::try_from(code).ok())
        {
            Some(code) => reason_of_code(code),
            None => UnavailableReason::Failed(error.to_string()),
        },
    }
}
