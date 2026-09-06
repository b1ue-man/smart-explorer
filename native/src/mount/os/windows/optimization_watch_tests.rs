//! Real ReadDirectoryChangesW coverage for the production remote-diff bridge.
use super::MountedOptimization;
use super::super::super::notifications::HostNotifications;
use crate::vfs::Backend;
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::windows::{fs::OpenOptionsExt, io::{AsRawHandle, FromRawHandle, OwnedHandle}},
    path::Path,
    ptr::null_mut,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{ERROR_IO_INCOMPLETE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    Storage::FileSystem::{
        ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED,
        FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_DIR_NAME,
        FILE_NOTIFY_CHANGE_FILE_NAME, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    },
    System::{IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        Threading::{CreateEventW, ResetEvent, WaitForSingleObject}},
};

pub(super) fn exercise(fixture: &MountedOptimization) -> io::Result<()> {
    let engine = &fixture.storage().context.engine;
    let directory = fixture.root()?.join("watch");
    assert_eq!(fs::read_dir(&directory)?.count(), 0);
    let mut notifications = HostNotifications::default();
    engine.refresh_metadata()?;
    notifications.deliver(engine, fixture.filesystem(), fixture.drive()?)?;
    let mut watcher = Watch::open(&directory)?;
    fixture.backend.put("/watch/note.md", b"before-external-change");
    await_change(fixture, &mut notifications, &mut watcher, 1)?;
    assert_eq!(fs::read(directory.join("note.md"))?, b"before-external-change");
    fixture.backend.put("/watch/note.md", b"after-external-change-longer");
    await_change(fixture, &mut notifications, &mut watcher, 3)?;
    assert_eq!(fs::read(directory.join("note.md"))?, b"after-external-change-longer");
    fixture.backend.remove_file("/watch/note.md")?;
    await_change(fixture, &mut notifications, &mut watcher, 2)?;
    assert!(matches!(fs::metadata(directory.join("note.md")),
        Err(error) if error.kind() == io::ErrorKind::NotFound));
    engine.refresh_metadata()?;
    notifications.deliver(engine, fixture.filesystem(), fixture.drive()?)?;
    assert!(watcher.poll(100)?.is_none(), "unchanged refresh generated a Windows notification");
    // Drop cancels and completes the outstanding request before unmount.
    drop(watcher);
    fixture.healthy()
}

fn await_change(fixture: &MountedOptimization, notifications: &mut HostNotifications,
    watcher: &mut Watch, expected_action: u32) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        let engine = &fixture.storage().context.engine;
        engine.refresh_metadata()?;
        notifications.deliver(engine, fixture.filesystem(), fixture.drive()?)?;
        if let Some(changes) = watcher.poll(50)? {
            eprintln!("[mount optimization] Windows watcher events={changes:?}");
            if changes.iter().any(|(action, name)| *action == expected_action && name == "note.md") {
                return Ok(());
            }
        }
        fixture.healthy()?;
    }
    Err(io::Error::other(format!("Windows watcher did not receive action={expected_action} for note.md")))
}

struct Watch {
    directory: File,
    event: OwnedHandle,
    overlapped: Box<OVERLAPPED>,
    // DWORD-aligned, under the documented 64-KiB network-buffer ceiling.
    buffer: Box<[u32; 4096]>,
    pending: bool,
}

impl Watch {
    fn open(path: &Path) -> io::Result<Self> {
        let directory = OpenOptions::new().access_mode(FILE_LIST_DIRECTORY)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED).open(path)?;
        let raw = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if raw.is_null() { return Err(io::Error::last_os_error()); }
        let event = unsafe { OwnedHandle::from_raw_handle(raw as _) };
        let mut overlapped: Box<OVERLAPPED> = Box::new(unsafe { std::mem::zeroed() });
        overlapped.hEvent = event.as_raw_handle() as _;
        let mut watcher = Self { directory, event, overlapped, buffer: Box::new([0; 4096]), pending: false };
        watcher.arm()?;
        Ok(watcher)
    }

    fn arm(&mut self) -> io::Result<()> {
        assert!(!self.pending);
        if unsafe { ResetEvent(self.event.as_raw_handle() as _) } == 0 {
            return Err(io::Error::last_os_error());
        }
        *self.overlapped = unsafe { std::mem::zeroed() };
        self.overlapped.hEvent = self.event.as_raw_handle() as _;
        // ATTRIBUTES is deliberate: pinned DokanNotifyUpdate does not emit
        // LAST_WRITE/SIZE. This is Node-style watcher compatibility, not an
        // assertion about every Obsidian version or every Windows filter.
        let filter = FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_ATTRIBUTES;
        if unsafe {
            ReadDirectoryChangesW(self.directory.as_raw_handle() as _, self.buffer.as_mut_ptr().cast(),
                std::mem::size_of_val(&*self.buffer) as u32, 0, filter, null_mut(), &mut *self.overlapped, None)
        } == 0 {
            return Err(io::Error::last_os_error());
        }
        self.pending = true;
        Ok(())
    }

    fn poll(&mut self, timeout_ms: u32) -> io::Result<Option<Vec<(u32, String)>>> {
        match unsafe { WaitForSingleObject(self.event.as_raw_handle() as _, timeout_ms) } {
            WAIT_TIMEOUT => return Ok(None),
            WAIT_OBJECT_0 => {}
            _ => return Err(io::Error::last_os_error()),
        }
        let mut bytes = 0;
        if unsafe { GetOverlappedResult(self.directory.as_raw_handle() as _, &*self.overlapped, &mut bytes, 0) } == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_IO_INCOMPLETE as i32) { self.pending = false; }
            return Err(error);
        }
        self.pending = false;
        let changes = self.decode(bytes as usize)?;
        self.arm()?;
        Ok(Some(changes))
    }

    fn decode(&self, length: usize) -> io::Result<Vec<(u32, String)>> {
        if length == 0 || length > std::mem::size_of_val(&*self.buffer) {
            return Err(io::Error::other("Windows notification buffer overflow or invalid length"));
        }
        let bytes = unsafe { std::slice::from_raw_parts(self.buffer.as_ptr().cast::<u8>(), length) };
        let mut offset = 0usize;
        let mut changes = Vec::new();
        loop {
            let header = bytes.get(offset..offset + 12).ok_or_else(|| io::Error::other("truncated notification"))?;
            let next = u32::from_le_bytes(header[0..4].try_into().expect("four bytes")) as usize;
            let action = u32::from_le_bytes(header[4..8].try_into().expect("four bytes"));
            let name_len = u32::from_le_bytes(header[8..12].try_into().expect("four bytes")) as usize;
            let end = (offset + 12).checked_add(name_len).ok_or_else(|| io::Error::other("notification length overflow"))?;
            let name = bytes.get(offset + 12..end).filter(|_| name_len % 2 == 0)
                .ok_or_else(|| io::Error::other("invalid notification filename length"))?;
            let units = name.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect::<Vec<_>>();
            let name = String::from_utf16(&units).map_err(|error| io::Error::other(error.to_string()))?;
            changes.push((action, name));
            if next == 0 { break; }
            if next % 4 != 0 || next < 12 + name_len {
                return Err(io::Error::other("invalid notification next-record offset"));
            }
            offset = offset.checked_add(next).ok_or_else(|| io::Error::other("notification offset overflow"))?;
        }
        Ok(changes)
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        if self.pending {
            // CancelIoEx requests cancellation; only completion permits freeing
            // OVERLAPPED/buffer. The outer process deadline also covers teardown.
            unsafe { CancelIoEx(self.directory.as_raw_handle() as _, &*self.overlapped); }
            let mut bytes = 0;
            unsafe { GetOverlappedResult(self.directory.as_raw_handle() as _, &*self.overlapped, &mut bytes, 1); }
            self.pending = false;
        }
    }
}
