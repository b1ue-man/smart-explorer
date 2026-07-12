use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};
use std::sync::Arc;

use super::io_adapters::FtpConnection;

fn io_err<E: std::fmt::Display>(error: E) -> io::Error {
    io::Error::other(error.to_string())
}

pub(super) trait FtpUpload: Send + Sync {
    fn upload(&self, path: &str, source: &mut File) -> io::Result<()>;
}

impl FtpUpload for FtpConnection {
    fn upload(&self, path: &str, source: &mut File) -> io::Result<()> {
        self.with_stream_mutation(|stream| {
            stream.put_file(path, source).map(|_| ()).map_err(io_err)
        })
    }
}

/// A remote writer whose only side-effect boundary is `flush`. Bytes are
/// staged in an anonymous file, so dropping an unfinished writer only removes
/// local temporary storage and never starts a remote STOR operation.
pub(super) struct FtpWriter {
    uploader: Arc<dyn FtpUpload>,
    path: String,
    spool: File,
    state: UploadState,
}

enum UploadState {
    Open,
    Committed,
    FailedAmbiguous(String),
}

impl FtpWriter {
    pub(super) fn new(uploader: Arc<dyn FtpUpload>, path: String) -> io::Result<Self> {
        Ok(Self {
            uploader,
            path,
            spool: tempfile::tempfile()?,
            state: UploadState::Open,
        })
    }

    fn commit(&mut self) -> io::Result<()> {
        match &self.state {
            UploadState::Committed => return Ok(()),
            UploadState::FailedAmbiguous(error) => {
                return Err(io_err(format!(
                    "FTP-Uploadstatus ist nach dem fehlgeschlagenen STOR unklar; der Upload wird nicht automatisch wiederholt: {error}"
                )))
            }
            UploadState::Open => {}
        }
        self.spool.flush()?;
        self.spool.seek(SeekFrom::Start(0))?;
        let result = self.uploader.upload(&self.path, &mut self.spool);
        match result {
            Ok(()) => {
                self.state = UploadState::Committed;
                Ok(())
            }
            Err(error) => {
                self.state = UploadState::FailedAmbiguous(error.to_string());
                self.spool.seek(SeekFrom::End(0))?;
                Err(error)
            }
        }
    }

    #[cfg(test)]
    fn spooled_bytes(&self) -> io::Result<u64> {
        self.spool.metadata().map(|metadata| metadata.len())
    }
}

impl Write for FtpWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        match &self.state {
            UploadState::Open => self.spool.write(data),
            UploadState::Committed => Err(io_err("Upload bereits abgeschlossen")),
            UploadState::FailedAmbiguous(_) => Err(io_err(
                "FTP-Uploadstatus ist unklar; weitere Daten werden nicht angenommen",
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

    impl FtpUpload for FailingUpload {
        fn upload(&self, _path: &str, _source: &mut File) -> io::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "server committed STOR but response was lost",
            ))
        }
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

    #[test]
    fn failed_upload_is_never_replayed_by_another_flush() {
        let upload = Arc::new(FailingUpload {
            calls: AtomicUsize::new(0),
        });
        let mut writer = FtpWriter::new(upload.clone(), "/ambiguous".to_string()).unwrap();
        writer.write_all(b"payload").unwrap();

        assert!(writer.flush().is_err());
        let second = writer.flush().unwrap_err();
        assert!(second.to_string().contains("nicht automatisch wiederholt"));
        assert_eq!(upload.calls.load(Ordering::SeqCst), 1);
        assert!(writer.write_all(b"late").is_err());
    }
}
