use super::io_err;
use std::io::{self, Read, Write};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime::Runtime;

/// `std::io::Read` over a tokio async read half, driven by the backend's runtime
/// (the protocol is request/response, so each call blocks one op — never nested
/// inside another `block_on`, since Backend calls run on app/scan threads).
pub(super) struct BlockingRead<R> {
    pub(super) rt: Arc<Runtime>,
    pub(super) inner: Option<R>,
}

impl<R: tokio::io::AsyncRead + Unpin + Send> Read for BlockingRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.inner.as_mut() {
            Some(inner) => self.rt.block_on(inner.read(buf)),
            None => Ok(0),
        }
    }
}

impl<R> Drop for BlockingRead<R> {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            // russh's ChannelCloseOnDrop uses tokio::spawn internally. Dropping
            // the split half inside this runtime avoids a delayed panic on the
            // plain std agent-reader thread after an earlier transport error.
            self.rt.block_on(async move {
                drop(inner);
            });
        }
    }
}

pub(super) struct BlockingWrite<W: tokio::io::AsyncWrite + Unpin + Send> {
    pub(super) rt: Arc<Runtime>,
    pub(super) inner: Option<W>,
}

impl<W: tokio::io::AsyncWrite + Unpin + Send> Write for BlockingWrite<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.inner.as_mut() {
            Some(inner) => self.rt.block_on(inner.write(buf)),
            None => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "SFTP exec stream already closed",
            )),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.inner.as_mut() {
            Some(inner) => self.rt.block_on(inner.flush()),
            None => Ok(()),
        }
    }
}

impl<W: tokio::io::AsyncWrite + Unpin + Send> Drop for BlockingWrite<W> {
    fn drop(&mut self) {
        // Closing the agent's stdin (channel EOF) is what makes the remote
        // `se-agent` exit, which then closes its stdout so the agent reader
        // thread unblocks and the whole bridge tears down cleanly. Best-effort.
        if let Some(mut inner) = self.inner.take() {
            self.rt.block_on(async move {
                let _ = inner.shutdown().await;
                drop(inner);
            });
        }
    }
}

pub(super) struct SftpReader {
    pub(super) rt: Arc<Runtime>,
    pub(super) file: russh_sftp::client::fs::File,
}

impl Read for SftpReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let rt = self.rt.clone();
        rt.block_on(async { self.file.read(buf).await })
    }
}

pub(super) struct SftpWriter {
    pub(super) rt: Arc<Runtime>,
    pub(super) file: Option<russh_sftp::client::fs::File>,
}

impl Write for SftpWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let rt = self.rt.clone();
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io_err("Datei geschlossen"))?;
        rt.block_on(async { file.write(buf).await })
    }

    fn flush(&mut self) -> io::Result<()> {
        let rt = self.rt.clone();
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io_err("Datei geschlossen"))?;
        rt.block_on(async { file.flush().await })
    }
}

impl Drop for SftpWriter {
    fn drop(&mut self) {
        // Ensure the remote file is flushed/closed (std::io::copy never calls
        // flush). Best-effort.
        if let Some(mut file) = self.file.take() {
            let rt = self.rt.clone();
            let _ = rt.block_on(async { file.shutdown().await });
        }
    }
}
