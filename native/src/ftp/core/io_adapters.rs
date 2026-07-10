use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use suppaftp::RustlsFtpStream;

fn io_err<E: std::fmt::Display>(error: E) -> io::Error {
    io::Error::other(error.to_string())
}

/// Owns the single FTP control connection. A streaming RETR checks the
/// connection out until its data stream is finalized or aborted; other calls
/// wait rather than issuing commands into an active transfer's response.
pub(super) struct FtpConnection {
    stream: Mutex<Option<RustlsFtpStream>>,
    available: Condvar,
}

impl FtpConnection {
    pub(super) fn new(stream: RustlsFtpStream) -> Self {
        Self {
            stream: Mutex::new(Some(stream)),
            available: Condvar::new(),
        }
    }

    fn wait_for_stream(&self) -> io::Result<MutexGuard<'_, Option<RustlsFtpStream>>> {
        let mut slot = self
            .stream
            .lock()
            .map_err(|_| io_err("FTP-Verbindung vergiftet"))?;
        while slot.is_none() {
            slot = self
                .available
                .wait(slot)
                .map_err(|_| io_err("FTP-Verbindung vergiftet"))?;
        }
        Ok(slot)
    }

    pub(super) fn with_stream<T>(
        &self,
        operation: impl FnOnce(&mut RustlsFtpStream) -> io::Result<T>,
    ) -> io::Result<T> {
        let mut slot = self.wait_for_stream()?;
        let stream = slot
            .as_mut()
            .ok_or_else(|| io_err("FTP-Verbindung ist nicht verfügbar"))?;
        operation(stream)
    }

    pub(super) fn open_reader(self: &Arc<Self>, path: &str) -> io::Result<FtpReader> {
        let mut slot = self.wait_for_stream()?;
        let data = slot
            .as_mut()
            .ok_or_else(|| io_err("FTP-Verbindung ist nicht verfügbar"))?
            .retr_as_stream(path)
            .map_err(io_err)?;
        let control = slot
            .take()
            .ok_or_else(|| io_err("FTP-Verbindung ist nicht verfügbar"))?;
        drop(slot);
        Ok(FtpReader {
            owner: self.clone(),
            control: Some(control),
            data: Some(Box::new(data)),
        })
    }

    fn return_stream(&self, stream: RustlsFtpStream) {
        let mut slot = match self.stream.lock() {
            Ok(slot) => slot,
            Err(poisoned) => poisoned.into_inner(),
        };
        debug_assert!(slot.is_none(), "FTP control connection returned twice");
        if slot.is_none() {
            *slot = Some(stream);
        }
        drop(slot);
        self.available.notify_one();
    }
}

pub(super) struct FtpReader {
    owner: Arc<FtpConnection>,
    control: Option<RustlsFtpStream>,
    data: Option<Box<dyn Read + Send>>,
}

impl FtpReader {
    fn close(&mut self, completed: bool) -> io::Result<()> {
        let Some(mut control) = self.control.take() else {
            return Ok(());
        };
        let result = match self.data.take() {
            Some(data) if completed => control.finalize_retr_stream(data).map_err(io_err),
            Some(data) => control.abort(data).map_err(io_err),
            None => Err(io_err("FTP-Datenstrom fehlt")),
        };
        self.owner.return_stream(control);
        result
    }
}

impl Read for FtpReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let Some(data) = self.data.as_mut() else {
            return Ok(0);
        };
        match data.read(buffer) {
            Ok(0) => self.close(true).map(|()| 0),
            Ok(read) => Ok(read),
            Err(error) => {
                let _ = self.close(false);
                Err(error)
            }
        }
    }
}

impl Drop for FtpReader {
    fn drop(&mut self) {
        if self.data.is_some() {
            let _ = self.close(false);
        }
    }
}

pub(super) trait FtpUpload: Send + Sync {
    fn upload(&self, path: &str, source: &mut File) -> io::Result<()>;
}

impl FtpUpload for FtpConnection {
    fn upload(&self, path: &str, source: &mut File) -> io::Result<()> {
        self.with_stream(|stream| stream.put_file(path, source).map(|_| ()).map_err(io_err))
    }
}

/// A remote writer whose only side-effect boundary is `flush`. Bytes are
/// staged in an anonymous file, so dropping an unfinished writer only removes
/// local temporary storage and never starts a remote STOR operation.
pub(super) struct FtpWriter {
    uploader: Arc<dyn FtpUpload>,
    path: String,
    spool: File,
    committed: bool,
}

impl FtpWriter {
    pub(super) fn new(uploader: Arc<dyn FtpUpload>, path: String) -> io::Result<Self> {
        Ok(Self {
            uploader,
            path,
            spool: tempfile::tempfile()?,
            committed: false,
        })
    }

    fn commit(&mut self) -> io::Result<()> {
        if self.committed {
            return Ok(());
        }
        self.spool.flush()?;
        self.spool.seek(SeekFrom::Start(0))?;
        let result = self.uploader.upload(&self.path, &mut self.spool);
        if result.is_ok() {
            self.committed = true;
            return Ok(());
        }
        self.spool.seek(SeekFrom::End(0))?;
        result
    }

    #[cfg(test)]
    fn spooled_bytes(&self) -> io::Result<u64> {
        self.spool.metadata().map(|metadata| metadata.len())
    }
}

impl Write for FtpWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.committed {
            return Err(io_err("Upload bereits abgeschlossen"));
        }
        self.spool.write(data)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    #[derive(Default)]
    struct CountingUpload {
        calls: AtomicUsize,
        bytes: AtomicU64,
    }

    impl FtpUpload for CountingUpload {
        fn upload(&self, _path: &str, source: &mut File) -> io::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let copied = io::copy(source, &mut io::sink())?;
            self.bytes.fetch_add(copied, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn dropping_unflushed_writer_never_uploads() {
        let upload = Arc::new(CountingUpload::default());
        {
            let mut writer = FtpWriter::new(upload.clone(), "/draft".to_string()).unwrap();
            writer.write_all(b"not committed").unwrap();
        }
        assert_eq!(upload.calls.load(Ordering::SeqCst), 0);
        assert_eq!(upload.bytes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn large_payload_is_disk_spooled_and_streamed_once() {
        static CHUNK: [u8; 64 * 1024] = [0x5a; 64 * 1024];
        const REPEATS: usize = 256;
        let upload = Arc::new(CountingUpload::default());
        let mut writer = FtpWriter::new(upload.clone(), "/large".to_string()).unwrap();
        for _ in 0..REPEATS {
            writer.write_all(&CHUNK).unwrap();
        }
        let expected = (CHUNK.len() * REPEATS) as u64;
        assert_eq!(writer.spooled_bytes().unwrap(), expected);

        writer.flush().unwrap();
        writer.flush().unwrap();
        assert_eq!(upload.calls.load(Ordering::SeqCst), 1);
        assert_eq!(upload.bytes.load(Ordering::SeqCst), expected);
        assert!(writer.write_all(b"late").is_err());
    }
}
