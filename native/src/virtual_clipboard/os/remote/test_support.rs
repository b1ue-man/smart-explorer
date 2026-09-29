//! Test support for remote virtual files: a small memory budget, an OLE
//! apartment guard, in-process objects and descriptor parsing the way
//! Explorer reads it. The fake remote is in `test_fakes.rs`.
use super::data_object::{Formats, RemoteDataObject};
use super::handoff::{Config, Handoff, Held, Memory};
use super::test_fakes::FakeRemote;
use super::worker::LifeToken;
use crate::transfer::ExternalSnapshot;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use windows::core::{Error, Result};
use windows::Win32::Foundation::{E_OUTOFMEMORY, E_UNEXPECTED, HGLOBAL, S_FALSE};
use windows::Win32::System::Com::{
    IDataObject, IStream, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, TYMED, TYMED_HGLOBAL,
    TYMED_ISTREAM,
};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize, ReleaseStgMedium};
use windows::Win32::UI::Shell::FILEDESCRIPTORW;

/// Generous for slow runners; waits end as soon as the condition holds.
pub(super) const DEADLINE: Duration = Duration::from_secs(20);

pub(super) fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + DEADLINE;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// OLE on the test thread (an STA, like the GUI thread).
pub(super) struct Apartment;

impl Apartment {
    pub(super) fn enter() -> Self {
        unsafe { OleInitialize(None) }.expect("OLE initializes on the test thread");
        Self
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { OleUninitialize() };
    }
}

/// The data object without a worker thread, called directly on the test
/// thread, and its shared state.
pub(super) fn in_process(remote: FakeRemote, config: Config) -> (IDataObject, Arc<Handoff>) {
    let handoff = Handoff::new(Arc::new(remote), config).expect("hand-off state");
    let life = LifeToken::detached(handoff.clone());
    let object = RemoteDataObject::new(handoff.clone(), life).into();
    (object, handoff)
}

/// A descriptor allocation Windows refuses (K19).
pub(super) fn refuse_alloc(_bytes: usize) -> Result<HGLOBAL> {
    Err(Error::from(E_OUTOFMEMORY))
}

/// A budget of a few files, to watch the prefetch stay inside it.
pub(super) struct TestBudget {
    capacity: u64,
    used: Mutex<u64>,
    max_used: AtomicU64,
    freed: Condvar,
}

impl TestBudget {
    pub(super) fn leaked(capacity: u64) -> &'static Self {
        Box::leak(Box::new(Self {
            capacity,
            used: Mutex::new(0),
            max_used: AtomicU64::new(0),
            freed: Condvar::new(),
        }))
    }

    pub(super) fn used(&self) -> u64 {
        *self.used.lock().unwrap()
    }

    pub(super) fn max_used(&self) -> u64 {
        self.max_used.load(Ordering::SeqCst)
    }
}

pub(super) struct TestMemory(pub(super) &'static TestBudget);

struct TestHeld {
    budget: &'static TestBudget,
    bytes: u64,
}

impl Drop for TestHeld {
    fn drop(&mut self) {
        *self.budget.used.lock().unwrap() -= self.bytes;
        self.budget.freed.notify_all();
    }
}

impl Memory for TestMemory {
    fn capacity(&self) -> u64 {
        self.0.capacity
    }

    fn reserve(&self, bytes: u64, cancel: &AtomicBool) -> Option<Held> {
        let budget = self.0;
        let bytes = bytes.min(budget.capacity);
        let mut used = budget.used.lock().unwrap();
        loop {
            if cancel.load(Ordering::Acquire) {
                return None;
            }
            if *used + bytes <= budget.capacity {
                *used += bytes;
                budget.max_used.fetch_max(*used, Ordering::SeqCst);
                return Some(Box::new(TestHeld { budget, bytes }));
            }
            used = budget
                .freed
                .wait_timeout(used, Duration::from_millis(20))
                .unwrap()
                .0;
        }
    }

    fn try_reserve(&self, bytes: u64) -> Option<Held> {
        let budget = self.0;
        let bytes = bytes.min(budget.capacity);
        let mut used = budget.used.lock().unwrap();
        if *used + bytes > budget.capacity {
            return None;
        }
        *used += bytes;
        budget.max_used.fetch_max(*used, Ordering::SeqCst);
        Some(Box::new(TestHeld { budget, bytes }))
    }
}

/// One descriptor entry as Explorer reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Described {
    pub(super) name: String,
    pub(super) flags: u32,
    pub(super) attributes: u32,
    pub(super) size: u64,
    pub(super) write_time: u64,
}

pub(super) fn request(format: u16, tymed: TYMED, lindex: i32) -> FORMATETC {
    FORMATETC {
        cfFormat: format,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex,
        tymed: tymed.0 as u32,
    }
}

pub(super) fn descriptor(object: &IDataObject, formats: &Formats) -> Result<Vec<Described>> {
    let mut medium: STGMEDIUM =
        unsafe { object.GetData(&request(formats.descriptor, TYMED_HGLOBAL, -1))? };
    let described = unsafe { parse(medium.u.hGlobal) };
    unsafe { ReleaseStgMedium(&mut medium) };
    Ok(described)
}

unsafe fn parse(handle: HGLOBAL) -> Vec<Described> {
    const ENTRY: usize = std::mem::size_of::<FILEDESCRIPTORW>();
    let size = GlobalSize(handle);
    let base = GlobalLock(handle).cast::<u8>();
    assert!(!base.is_null(), "descriptor block locks");
    let count = std::ptr::read_unaligned(base.cast::<u32>()) as usize;
    assert!(
        size >= 4 + count.max(1) * ENTRY,
        "descriptor block holds every entry"
    );
    let described = (0..count)
        .map(|index| {
            let entry =
                std::ptr::read_unaligned(base.add(4 + index * ENTRY).cast::<FILEDESCRIPTORW>());
            let units = entry.cFileName;
            let length = units
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(units.len());
            let written = entry.ftLastWriteTime;
            Described {
                name: String::from_utf16(&units[..length]).expect("valid UTF-16 name"),
                flags: entry.dwFlags,
                attributes: entry.dwFileAttributes,
                size: (u64::from(entry.nFileSizeHigh) << 32) | u64::from(entry.nFileSizeLow),
                write_time: (u64::from(written.dwHighDateTime) << 32)
                    | u64::from(written.dwLowDateTime),
            }
        })
        .collect();
    let _ = GlobalUnlock(handle);
    described
}

pub(super) fn contents(object: &IDataObject, formats: &Formats, index: i32) -> Result<IStream> {
    let mut medium: STGMEDIUM =
        unsafe { object.GetData(&request(formats.contents, TYMED_ISTREAM, index))? };
    let stream = unsafe { (*medium.u.pstm).clone() };
    unsafe { ReleaseStgMedium(&mut medium) };
    stream.ok_or_else(|| Error::from(E_UNEXPECTED))
}

/// Reads until the short read that ends the stream, in odd-sized chunks.
pub(super) fn read_all(stream: &IStream) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    let mut chunk = [0u8; 7_919];
    loop {
        let mut read = 0u32;
        let status = unsafe {
            stream.Read(
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                Some(&mut read),
            )
        };
        status.ok()?;
        data.extend_from_slice(&chunk[..read as usize]);
        if status == S_FALSE {
            return Ok(data);
        }
    }
}

pub(super) fn read_exact(stream: &IStream, count: usize) -> Vec<u8> {
    let mut data = vec![0u8; count];
    let mut read = 0u32;
    let status = unsafe { stream.Read(data.as_mut_ptr().cast(), count as u32, Some(&mut read)) };
    assert!(status.is_ok(), "read failed: {status:?}");
    data.truncate(read as usize);
    data
}

/// The newest transfer-list entry with `label`.
pub(super) fn external(label: &str) -> ExternalSnapshot {
    crate::transfer::external_snapshots()
        .into_iter()
        .rev()
        .find(|snapshot| snapshot.label == label)
        .expect("the hand-off appears in the transfer list")
}
