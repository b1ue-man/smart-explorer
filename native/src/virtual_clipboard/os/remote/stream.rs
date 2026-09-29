//! One FILECONTENTS stream: Explorer reads a remote file through it while
//! the fetch fills from the connection. Sequential at heart; a forward seek
//! skips bytes, a backward seek (or Clone) fetches the file again.
use super::catalog::Catalog;
use super::fetch::{Fetch, FetchError, FetchHandle};
use super::handoff::Handoff;
use super::worker::LifeToken;
use crate::transfer::ListedEntry;
use std::ffi::c_void;
use std::sync::{Arc, Mutex, MutexGuard};
use windows::core::{implement, Error, Result, HRESULT, PWSTR};
use windows::Win32::Foundation::{
    E_OUTOFMEMORY, E_POINTER, STG_E_ACCESSDENIED, STG_E_INVALIDFUNCTION, STG_E_WRITEFAULT, S_FALSE,
    S_OK,
};
use windows::Win32::System::Com::{
    CoTaskMemAlloc, ISequentialStream, ISequentialStream_Impl, IStream, IStream_Impl, LOCKTYPE,
    STATFLAG, STATFLAG_NONAME, STATSTG, STGC, STGM_READ, STGTY_STREAM, STREAM_SEEK,
    STREAM_SEEK_CUR, STREAM_SEEK_END, STREAM_SEEK_SET,
};

/// CopyTo moves data in blocks of the fetch's network read size.
const COPY_BLOCK: usize = 256 << 10;
/// Fresh fetches per Read after a failure: a connection that broke mid-file
/// continues where it broke without Explorer noticing; one that fails again
/// at once reports its error instead of retrying blindly.
const RENEWALS: u32 = 1;

struct Cursor {
    fetch: Option<FetchHandle>,
    /// Position Explorer asked for.
    position: u64,
    /// Bytes taken from `fetch` so far.
    taken: u64,
}

#[implement(IStream, ISequentialStream)]
pub(super) struct RemoteStream {
    handoff: Arc<Handoff>,
    index: usize,
    entry: ListedEntry,
    cursor: Mutex<Cursor>,
    life: Arc<LifeToken>,
}

fn to_error(error: FetchError) -> Error {
    Error::new(error.code, error.message)
}

impl RemoteStream {
    pub(super) fn open(
        handoff: &Arc<Handoff>,
        catalog: &Arc<Catalog>,
        index: usize,
        life: Arc<LifeToken>,
    ) -> Result<Self> {
        let entry = catalog
            .entries
            .get(index)
            .cloned()
            .ok_or_else(|| Error::from(STG_E_INVALIDFUNCTION))?;
        let fetch = handoff.open_fetch(catalog, index, &entry)?;
        Ok(Self::with(handoff.clone(), index, entry, fetch, 0, life))
    }

    fn with(
        handoff: Arc<Handoff>,
        index: usize,
        entry: ListedEntry,
        fetch: Option<FetchHandle>,
        position: u64,
        life: Arc<LifeToken>,
    ) -> Self {
        handoff.sessions.stream_opened();
        Self {
            handoff,
            index,
            entry,
            cursor: Mutex::new(Cursor {
                fetch,
                position,
                taken: 0,
            }),
            life,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Cursor> {
        self.cursor
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The fetch to read from, how far to skip first, and whether it replaces
    /// a failed one. A fresh fetch starts at the position when there is none
    /// (Clone), when Explorer moved back, or (if `renew` allows) when the
    /// current one failed and has nothing left to deliver.
    fn prepare(&self, renew: bool) -> Result<(Arc<Fetch>, u64, bool)> {
        let mut cursor = self.lock();
        let (fresh, renewed) = match &cursor.fetch {
            Some(_) if cursor.position < cursor.taken => (true, false),
            Some(fetch) if fetch.failed() => (renew, renew),
            Some(_) => (false, false),
            None => (true, false),
        };
        if fresh {
            let start = cursor.position;
            // The old fetch (if any) stops when its handle drops here.
            cursor.fetch = Some(self.handoff.demand_fetch(&self.entry, start)?);
            cursor.taken = start;
        }
        let fetch = cursor
            .fetch
            .as_ref()
            .map(FetchHandle::shared)
            .ok_or_else(|| Error::from(STG_E_INVALIDFUNCTION))?;
        Ok((fetch, cursor.position - cursor.taken, renewed))
    }

    /// Records bytes taken from `fetch` unless a concurrent rewind replaced
    /// it; a seek target further ahead stays the position.
    fn advance(&self, fetch: &Arc<Fetch>, bytes: u64) {
        let mut cursor = self.lock();
        if matches!(&cursor.fetch, Some(current) if std::ptr::eq(&**current, &**fetch)) {
            cursor.taken += bytes;
            cursor.position = cursor.position.max(cursor.taken);
        }
    }

    /// Fills `out` unless the file ends first; short counts mean the end.
    fn read_into(&self, out: &mut [u8]) -> Result<usize> {
        if self.entry.is_dir {
            return Ok(0);
        }
        self.handoff.prefetch.touch();
        let origin = self.lock().position;
        let (mut filled, mut ended, mut renewals) = (0, false, 0);
        while filled < out.len() && !ended {
            let (fetch, skip, renewed) = self.prepare(renewals < RENEWALS)?;
            renewals += u32::from(renewed);
            let step = if skip > 0 {
                fetch.skip(skip)
            } else {
                fetch.read(&mut out[filled..])
            };
            match step {
                Ok(taken) => {
                    if skip == 0 {
                        filled += taken.count;
                    }
                    self.advance(&fetch, taken.count as u64);
                    // A short take without the end leaves an error pending:
                    // the next round continues with a fresh fetch.
                    ended = taken.ended;
                }
                // Nothing buffered is left: the next round starts a fresh fetch.
                Err(_) if renewals < RENEWALS && fetch.failed() => {}
                Err(error) => {
                    // Explorer may repeat the Read: it starts from the same
                    // position, not after bytes it never received.
                    self.lock().position = origin;
                    return Err(self.failed(error));
                }
            }
        }
        let taken = self.lock().taken;
        self.handoff.sessions.bytes(filled as u64);
        let whole = self.entry.size_known && taken >= self.entry.size;
        if ended || whole {
            self.handoff.sessions.delivered(self.index);
        }
        Ok(filled)
    }

    fn failed(&self, error: FetchError) -> Error {
        self.handoff
            .sessions
            .failed(self.index, &self.entry.rel, &error.message);
        to_error(error)
    }

    fn size(&self) -> Result<u64> {
        if self.entry.is_dir {
            return Ok(0);
        }
        if self.entry.size_known {
            return Ok(self.entry.size);
        }
        let (fetch, _, _) = self.prepare(true)?;
        fetch.total().map_err(to_error)
    }

    /// IStream::CopyTo: up to `count` bytes into `target`; `totals` counts
    /// the bytes read and written even when it fails.
    fn copy_to(&self, target: &IStream, count: u64, totals: &mut (u64, u64)) -> Result<()> {
        let mut block = vec![0u8; COPY_BLOCK.min(usize::try_from(count).unwrap_or(COPY_BLOCK))];
        while totals.0 < count && !block.is_empty() {
            let want = block
                .len()
                .min(usize::try_from(count - totals.0).unwrap_or(usize::MAX));
            let read = self.read_into(&mut block[..want])?;
            if read == 0 {
                break;
            }
            totals.0 += read as u64;
            let mut sent = 0u32;
            unsafe { target.Write(block.as_ptr().cast(), read as u32, Some(&mut sent)) }.ok()?;
            totals.1 += u64::from(sent);
            if sent as usize != read {
                return Err(STG_E_WRITEFAULT.into());
            }
            if read < want {
                break;
            }
        }
        Ok(())
    }

    fn name(&self) -> Result<PWSTR> {
        let name = self.entry.rel.rsplit('/').next().unwrap_or("FileContents");
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let memory = unsafe { CoTaskMemAlloc(wide.len() * 2) }.cast::<u16>();
        if memory.is_null() {
            return Err(E_OUTOFMEMORY.into());
        }
        // SAFETY: the allocation holds exactly `wide.len()` units.
        unsafe { std::ptr::copy_nonoverlapping(wide.as_ptr(), memory, wide.len()) };
        Ok(PWSTR(memory))
    }
}

impl Drop for RemoteStream {
    fn drop(&mut self) {
        self.handoff.sessions.stream_closed();
    }
}

impl ISequentialStream_Impl for RemoteStream_Impl {
    fn Read(&self, pv: *mut c_void, cb: u32, pcbread: *mut u32) -> HRESULT {
        if let Some(read) = unsafe { pcbread.as_mut() } {
            *read = 0;
        }
        if cb == 0 {
            return S_OK;
        }
        if pv.is_null() {
            return E_POINTER;
        }
        // COM supplies a writable buffer of `cb` bytes for this call only.
        let out = unsafe { std::slice::from_raw_parts_mut(pv.cast::<u8>(), cb as usize) };
        match self.read_into(out) {
            Ok(count) => {
                if let Some(read) = unsafe { pcbread.as_mut() } {
                    *read = count as u32;
                }
                // A short read is the documented end of the stream.
                if count == out.len() {
                    S_OK
                } else {
                    S_FALSE
                }
            }
            Err(error) => error.code(),
        }
    }

    fn Write(&self, _pv: *const c_void, _cb: u32, pcbwritten: *mut u32) -> HRESULT {
        if let Some(written) = unsafe { pcbwritten.as_mut() } {
            *written = 0;
        }
        STG_E_ACCESSDENIED
    }
}

impl IStream_Impl for RemoteStream_Impl {
    fn Seek(&self, dlibmove: i64, dworigin: STREAM_SEEK, plibnewposition: *mut u64) -> Result<()> {
        let base = match dworigin {
            STREAM_SEEK_SET => 0,
            STREAM_SEEK_CUR => self.lock().position,
            STREAM_SEEK_END => self.size()?,
            _ => return Err(STG_E_INVALIDFUNCTION.into()),
        };
        let next = u64::try_from(i128::from(base) + i128::from(dlibmove))
            .map_err(|_| Error::from(STG_E_INVALIDFUNCTION))?;
        // Moving is lazy: the next Read skips ahead or fetches again.
        self.lock().position = next;
        if let Some(result) = unsafe { plibnewposition.as_mut() } {
            *result = next;
        }
        Ok(())
    }

    fn SetSize(&self, _libnewsize: u64) -> Result<()> {
        Err(STG_E_ACCESSDENIED.into())
    }

    fn CopyTo(
        &self,
        pstm: Option<&IStream>,
        cb: u64,
        pcbread: *mut u64,
        pcbwritten: *mut u64,
    ) -> Result<()> {
        let mut totals = (0u64, 0u64);
        let result = match pstm {
            Some(target) => self.copy_to(target, cb, &mut totals),
            None => Err(E_POINTER.into()),
        };
        unsafe {
            if let Some(read) = pcbread.as_mut() {
                *read = totals.0;
            }
            if let Some(written) = pcbwritten.as_mut() {
                *written = totals.1;
            }
        }
        result
    }

    fn Commit(&self, _grfcommitflags: &STGC) -> Result<()> {
        Ok(())
    }

    fn Revert(&self) -> Result<()> {
        Err(STG_E_INVALIDFUNCTION.into())
    }

    fn LockRegion(&self, _liboffset: u64, _cb: u64, _dwlocktype: &LOCKTYPE) -> Result<()> {
        Err(STG_E_INVALIDFUNCTION.into())
    }

    fn UnlockRegion(&self, _liboffset: u64, _cb: u64, _dwlocktype: u32) -> Result<()> {
        Err(STG_E_INVALIDFUNCTION.into())
    }

    fn Stat(&self, pstatstg: *mut STATSTG, grfstatflag: &STATFLAG) -> Result<()> {
        if pstatstg.is_null() {
            return Err(E_POINTER.into());
        }
        let mut stat = STATSTG {
            r#type: STGTY_STREAM.0 as u32,
            cbSize: self.size()?,
            grfMode: STGM_READ,
            ..Default::default()
        };
        if grfstatflag.0 & STATFLAG_NONAME.0 == 0 {
            stat.pwcsName = self.name()?;
        }
        unsafe { pstatstg.write(stat) };
        Ok(())
    }

    fn Clone(&self) -> Result<IStream> {
        let position = self.lock().position;
        Ok(RemoteStream::with(
            self.handoff.clone(),
            self.index,
            self.entry.clone(),
            None,
            position,
            self.life.clone(),
        )
        .into())
    }
}
