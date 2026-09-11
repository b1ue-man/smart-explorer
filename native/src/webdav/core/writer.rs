use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};
use std::sync::Arc;

#[cfg(test)]
#[path = "copy_writer_task_tests.rs"]
mod copy_writer_task_tests;

fn io_err<E: std::fmt::Display>(error: E) -> io::Error {
    io::Error::other(error.to_string())
}

fn request_err(error: ureq::Error, create_new: bool) -> io::Error {
    let kind = if create_new && matches!(&error, ureq::Error::Status(412, _)) {
        io::ErrorKind::AlreadyExists
    } else {
        io::ErrorKind::Other
    };
    io::Error::new(kind, error.to_string())
}

trait WebdavUpload: Send + Sync {
    fn upload(&self, url: &str, auth: &str, length: u64, source: &mut File) -> io::Result<()>;
}

struct UreqUpload {
    agent: ureq::Agent,
    create_new: bool,
}

impl WebdavUpload for UreqUpload {
    fn upload(&self, url: &str, auth: &str, length: u64, source: &mut File) -> io::Result<()> {
        let request = self
            .agent
            .put(url)
            .set("Content-Length", &length.to_string());
        // RFC 9110 section 13.1.2: the server must reject an occupied
        // resource without applying this PUT. A prior probe cannot do this.
        let request = if self.create_new {
            request.set("If-None-Match", "*")
        } else {
            request
        };
        let request = if auth.is_empty() {
            request
        } else {
            request.set("Authorization", auth)
        };
        let response = request
            .send(source)
            .map_err(|error| request_err(error, self.create_new))?;
        let status = response.status();
        let committed = if self.create_new {
            // A successfully created PUT representation must return 201.
            // In particular, 202 is not an acknowledgement of completion.
            status == 201
        } else {
            (200..300).contains(&status) && status != 207
        };
        if committed {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "WebDAV PUT returned unexpected HTTP status {status}"
            )))
        }
    }
}

/// Buffers to an anonymous disk file and performs PUT only when `flush` is
/// called. Dropping an unfinished writer cannot create a partial remote file.
pub(super) struct WebdavWriter {
    uploader: Arc<dyn WebdavUpload>,
    url: String,
    auth: String,
    spool: File,
    state: UploadState,
}

enum UploadState {
    Open,
    Committed,
    Failed {
        kind: io::ErrorKind,
        message: String,
    },
}

impl WebdavWriter {
    pub(super) fn new(agent: ureq::Agent, url: String, auth: String) -> io::Result<Self> {
        Self::with_mode(agent, url, auth, false)
    }

    pub(super) fn new_exclusive(
        agent: ureq::Agent,
        url: String,
        auth: String,
    ) -> io::Result<Self> {
        Self::with_mode(agent, url, auth, true)
    }

    fn with_mode(
        agent: ureq::Agent,
        url: String,
        auth: String,
        create_new: bool,
    ) -> io::Result<Self> {
        Ok(Self {
            uploader: Arc::new(UreqUpload { agent, create_new }),
            url,
            auth,
            spool: tempfile::tempfile()?,
            state: UploadState::Open,
        })
    }

    #[cfg(test)]
    fn with_uploader(uploader: Arc<dyn WebdavUpload>) -> io::Result<Self> {
        Ok(Self {
            uploader,
            url: "http://unused.test/file".to_string(),
            auth: String::new(),
            spool: tempfile::tempfile()?,
            state: UploadState::Open,
        })
    }

    fn commit(&mut self) -> io::Result<()> {
        match &self.state {
            UploadState::Committed => return Ok(()),
            UploadState::Failed { kind, message } => {
                return Err(io::Error::new(*kind, format!(
                    "WebDAV-PUT fehlgeschlagen; der Upload wird nicht automatisch wiederholt: {message}"
                )))
            }
            UploadState::Open => {}
        }
        self.spool.flush()?;
        let length = self.spool.metadata()?.len();
        self.spool.seek(SeekFrom::Start(0))?;
        let result = self
            .uploader
            .upload(&self.url, &self.auth, length, &mut self.spool);
        match result {
            Ok(()) => {
                self.state = UploadState::Committed;
                Ok(())
            }
            Err(error) => {
                self.state = UploadState::Failed {
                    kind: error.kind(),
                    message: error.to_string(),
                };
                Err(error)
            }
        }
    }

    #[cfg(test)]
    fn spooled_bytes(&self) -> io::Result<u64> {
        self.spool.metadata().map(|metadata| metadata.len())
    }
}

impl Write for WebdavWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        match &self.state {
            UploadState::Open => self.spool.write(data),
            UploadState::Committed => Err(io_err("Upload bereits abgeschlossen")),
            UploadState::Failed { kind, .. } => Err(io::Error::new(
                *kind,
                "WebDAV-PUT fehlgeschlagen; weitere Daten werden nicht angenommen",
            )),
        }
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

    struct FailingUpload {
        calls: AtomicUsize,
    }

    impl WebdavUpload for FailingUpload {
        fn upload(
            &self,
            _url: &str,
            _auth: &str,
            _length: u64,
            _source: &mut File,
        ) -> io::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "server committed PUT but response was lost",
            ))
        }
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

    #[test]
    fn failed_put_is_never_replayed_by_another_flush() {
        let upload = Arc::new(FailingUpload {
            calls: AtomicUsize::new(0),
        });
        let mut writer = WebdavWriter::with_uploader(upload.clone()).unwrap();
        writer.write_all(b"payload").unwrap();

        assert!(writer.flush().is_err());
        let second = writer.flush().unwrap_err();
        assert!(second.to_string().contains("nicht automatisch wiederholt"));
        assert_eq!(upload.calls.load(Ordering::SeqCst), 1);
        assert!(writer.write_all(b"late").is_err());
    }
}
