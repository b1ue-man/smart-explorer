//! Read-only COM streams retain a granted file handle. Clones share bytes,
//! with independent cursors, so Explorer can seek without reopening the path.
use std::{
    ffi::c_void,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    sync::{Arc, Mutex},
};
use windows::core::{implement, Error, Result, HRESULT, PWSTR};
use windows::Win32::{
    Foundation::{
        E_OUTOFMEMORY, E_POINTER, STG_E_ACCESSDENIED, STG_E_INVALIDFUNCTION, STG_E_READFAULT,
        STG_E_WRITEFAULT, S_FALSE, S_OK,
    },
    System::Com::{
        CoTaskMemAlloc, ISequentialStream, ISequentialStream_Impl, IStream, IStream_Impl, LOCKTYPE,
        STATFLAG, STATFLAG_NONAME, STATSTG, STGC, STGM_READ, STGTY_STREAM, STREAM_SEEK,
        STREAM_SEEK_CUR, STREAM_SEEK_END, STREAM_SEEK_SET,
    },
};

#[implement(IStream, ISequentialStream)]
struct ReadStream {
    file: Arc<Mutex<File>>,
    position: Mutex<u64>,
}

pub(super) fn open(path: &str) -> Result<IStream> {
    let file = crate::local_access::open_read(Path::new(path)).map_err(com_error)?;
    Ok(ReadStream {
        file: Arc::new(Mutex::new(file)),
        position: Mutex::new(0),
    }
    .into())
}

fn com_error(error: io::Error) -> Error {
    let code = error
        .raw_os_error()
        .map(|code| HRESULT::from_win32(code as u32))
        .unwrap_or(STG_E_READFAULT);
    Error::new(code, error.to_string())
}

impl ReadStream {
    fn read_bytes(&self, buffer: &mut [u8]) -> Result<usize> {
        let mut position = self
            .position
            .lock()
            .map_err(|_| Error::from(STG_E_READFAULT))?;
        let mut file = self.file.lock().map_err(|_| Error::from(STG_E_READFAULT))?;
        file.seek(SeekFrom::Start(*position)).map_err(com_error)?;
        // Read as much as requested; a short COM read denotes end-of-stream.
        let mut count = 0;
        while count < buffer.len() {
            match file.read(&mut buffer[count..]) {
                Ok(0) => break,
                Ok(read) => {
                    count += read;
                    *position += read as u64;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(com_error(error)),
            }
        }
        Ok(count)
    }
}

impl ISequentialStream_Impl for ReadStream_Impl {
    fn Read(&self, buffer: *mut c_void, count: u32, read: *mut u32) -> HRESULT {
        unsafe {
            if !read.is_null() {
                *read = 0;
            }
        }
        if count == 0 {
            return S_OK;
        }
        if buffer.is_null() {
            return E_POINTER;
        }
        // COM supplies a writable buffer of `count` bytes for this call only.
        let buffer = unsafe { std::slice::from_raw_parts_mut(buffer.cast::<u8>(), count as usize) };
        match self.read_bytes(buffer) {
            Ok(bytes) => {
                unsafe {
                    if !read.is_null() {
                        *read = bytes as u32;
                    }
                }
                if bytes == count as usize {
                    S_OK
                } else {
                    S_FALSE
                }
            }
            Err(error) => error.code(),
        }
    }

    fn Write(&self, _: *const c_void, _: u32, written: *mut u32) -> HRESULT {
        unsafe {
            if !written.is_null() {
                *written = 0;
            }
        }
        STG_E_ACCESSDENIED
    }
}

impl IStream_Impl for ReadStream_Impl {
    fn Seek(&self, offset: i64, origin: STREAM_SEEK, result: *mut u64) -> Result<()> {
        let mut position = self
            .position
            .lock()
            .map_err(|_| Error::from(STG_E_READFAULT))?;
        let base = match origin {
            STREAM_SEEK_SET => 0,
            STREAM_SEEK_CUR => *position,
            STREAM_SEEK_END => self
                .file
                .lock()
                .map_err(|_| Error::from(STG_E_READFAULT))?
                .metadata()
                .map_err(com_error)?
                .len(),
            _ => return Err(STG_E_INVALIDFUNCTION.into()),
        };
        let next = i128::from(base) + i128::from(offset);
        *position = u64::try_from(next).map_err(|_| Error::from(STG_E_INVALIDFUNCTION))?;
        unsafe {
            if !result.is_null() {
                *result = *position;
            }
        }
        Ok(())
    }

    fn SetSize(&self, _: u64) -> Result<()> {
        Err(STG_E_ACCESSDENIED.into())
    }

    fn CopyTo(
        &self,
        target: Option<&IStream>,
        count: u64,
        read: *mut u64,
        written: *mut u64,
    ) -> Result<()> {
        unsafe {
            if !read.is_null() {
                *read = 0;
            }
            if !written.is_null() {
                *written = 0;
            }
        }
        let target = target.ok_or_else(|| Error::from(E_POINTER))?;
        let mut remaining = count;
        let mut buffer = [0u8; 64 * 1024];
        while remaining > 0 {
            let length = remaining.min(buffer.len() as u64) as usize;
            let bytes = self.read_bytes(&mut buffer[..length])?;
            if bytes == 0 {
                break;
            }
            let mut sent = 0;
            let status =
                unsafe { target.Write(buffer.as_ptr().cast(), bytes as u32, Some(&mut sent)) };
            unsafe {
                if !read.is_null() {
                    *read += bytes as u64;
                }
                if !written.is_null() {
                    *written += u64::from(sent);
                }
            }
            status.ok()?;
            if sent as usize != bytes {
                return Err(STG_E_WRITEFAULT.into());
            }
            remaining -= bytes as u64;
        }
        Ok(())
    }

    fn Commit(&self, _: &STGC) -> Result<()> {
        Ok(())
    }
    fn Revert(&self) -> Result<()> {
        Err(STG_E_INVALIDFUNCTION.into())
    }
    fn LockRegion(&self, _: u64, _: u64, _: &LOCKTYPE) -> Result<()> {
        Err(STG_E_INVALIDFUNCTION.into())
    }
    fn UnlockRegion(&self, _: u64, _: u64, _: u32) -> Result<()> {
        Err(STG_E_INVALIDFUNCTION.into())
    }

    fn Stat(&self, result: *mut STATSTG, flags: &STATFLAG) -> Result<()> {
        if result.is_null() {
            return Err(E_POINTER.into());
        }
        let metadata = self
            .file
            .lock()
            .map_err(|_| Error::from(STG_E_READFAULT))?
            .metadata()
            .map_err(com_error)?;
        let mut stat = STATSTG {
            r#type: STGTY_STREAM.0 as u32,
            cbSize: metadata.len(),
            grfMode: STGM_READ,
            ..Default::default()
        };
        if flags.0 & STATFLAG_NONAME.0 == 0 {
            let name: Vec<u16> = "FileContents\0".encode_utf16().collect();
            let memory = unsafe { CoTaskMemAlloc(name.len() * 2) }.cast::<u16>();
            if memory.is_null() {
                return Err(E_OUTOFMEMORY.into());
            }
            unsafe {
                std::ptr::copy_nonoverlapping(name.as_ptr(), memory, name.len());
            }
            stat.pwcsName = PWSTR(memory);
        }
        unsafe {
            *result = stat;
        }
        Ok(())
    }

    fn Clone(&self) -> Result<IStream> {
        let position = *self
            .position
            .lock()
            .map_err(|_| Error::from(STG_E_READFAULT))?;
        Ok(ReadStream {
            file: self.file.clone(),
            position: Mutex::new(position),
        }
        .into())
    }
}
