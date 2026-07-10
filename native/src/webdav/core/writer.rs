use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};
use std::sync::Arc;

fn io_err<E: std::fmt::Display>(error: E) -> io::Error {
    io::Error::other(error.to_string())
}

fn request_err(error: ureq::Error) -> io::Error {
    io::Error::other(error.to_string())
}

trait WebdavUpload: Send + Sync {
    fn upload(&self, url: &str, auth: &str, length: u64, source: &mut File) -> io::Result<()>;
}

struct UreqUpload {
    agent: ureq::Agent,
}

impl WebdavUpload for UreqUpload {
    fn upload(&self, url: &str, auth: &str, length: u64, source: &mut File) -> io::Result<()> {
        let request = self
            .agent
            .put(url)
            .set("Content-Length", &length.to_string());
        let request = if auth.is_empty() {
            request
        } else {
            request.set("Authorization", auth)
        };
        request.send(source).map_err(request_err)?;
        Ok(())
    }
}

/// Buffers to an anonymous disk file and performs PUT only when `flush` is
/// called. Dropping an unfinished writer cannot create a partial remote file.
pub(super) struct WebdavWriter {
    uploader: Arc<dyn WebdavUpload>,
    url: String,
    auth: String,
    spool: File,
    committed: bool,
}

impl WebdavWriter {
    pub(super) fn new(agent: ureq::Agent, url: String, auth: String) -> io::Result<Self> {
        Ok(Self {
            uploader: Arc::new(UreqUpload { agent }),
            url,
            auth,
            spool: tempfile::tempfile()?,
            committed: false,
        })
    }

    #[cfg(test)]
    fn with_uploader(uploader: Arc<dyn WebdavUpload>) -> io::Result<Self> {
        Ok(Self {
            uploader,
            url: "http://unused.test/file".to_string(),
            auth: String::new(),
            spool: tempfile::tempfile()?,
            committed: false,
        })
    }

    fn commit(&mut self) -> io::Result<()> {
        if self.committed {
            return Ok(());
        }
        self.spool.flush()?;
        let length = self.spool.metadata()?.len();
        self.spool.seek(SeekFrom::Start(0))?;
        let result = self
            .uploader
            .upload(&self.url, &self.auth, length, &mut self.spool);
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

impl Write for WebdavWriter {
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

    impl WebdavUpload for CountingUpload {
        fn upload(
            &self,
            _url: &str,
            _auth: &str,
            length: u64,
            source: &mut File,
        ) -> io::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let copied = io::copy(source, &mut io::sink())?;
            assert_eq!(copied, length);
            self.bytes.fetch_add(copied, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn dropping_unflushed_writer_never_sends_put() {
        let upload = Arc::new(CountingUpload::default());
        {
            let mut writer = WebdavWriter::with_uploader(upload.clone()).unwrap();
            writer.write_all(b"not committed").unwrap();
        }
        assert_eq!(upload.calls.load(Ordering::SeqCst), 0);
        assert_eq!(upload.bytes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn large_payload_is_disk_spooled_and_streamed_once() {
        static CHUNK: [u8; 64 * 1024] = [0xa5; 64 * 1024];
        const REPEATS: usize = 256;
        let upload = Arc::new(CountingUpload::default());
        let mut writer = WebdavWriter::with_uploader(upload.clone()).unwrap();
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
