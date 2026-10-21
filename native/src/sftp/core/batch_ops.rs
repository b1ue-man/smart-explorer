//! The wire under the SFTP packets (`batch_put.rs`, `batch_get.rs`): the
//! requests of one pool channel as futures, so the packet logic overlaps the
//! steps of many small files and can be driven against a script in tests.
//! One channel is one `sftp-server`, which answers its requests in order; the
//! round trips of many files overlap because their requests are all on the
//! wire before the first answer comes back.
use super::backend::{create_new_flags, SftpBackend};
use super::channel_pool::{note_failure, open_failed, PoolChannel};
use super::connection::SftpConnection;
use super::io_err;
use crate::vfs::BatchLimits;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, OpenFlags, StatusCode};
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;

/// A failed request. `answered`: the server replied with a status (or the
/// request never left the client), so the request certainly changed
/// nothing; `false`: transport failure or timeout, the result is unknown.
pub(super) struct WireError {
    pub error: io::Error,
    pub answered: bool,
}

/// Not `Send`: it is polled on the thread of the caller's `block_on`.
pub(super) type WireFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, WireError>> + 'a>>;

pub(super) trait BatchWire {
    /// OPEN with WRITE | CREATE | EXCL; returns the handle.
    fn open_new(&self, path: &str) -> WireFuture<'_, String>;
    fn write(&self, handle: &str, offset: u64, data: Vec<u8>) -> WireFuture<'_, ()>;
    fn close(&self, handle: &str) -> WireFuture<'_, ()>;
    /// SFTP v3 RENAME, which refuses an existing target.
    fn rename(&self, from: &str, to: &str) -> WireFuture<'_, ()>;
    fn remove(&self, path: &str) -> WireFuture<'_, ()>;
    /// LSTAT: whether anything of any kind is there, links not followed.
    fn exists(&self, path: &str) -> WireFuture<'_, bool>;
    fn open_read(&self, path: &str) -> WireFuture<'_, String>;
    /// An empty answer is the end of the file.
    fn read(&self, handle: &str, offset: u64, len: u32) -> WireFuture<'_, Vec<u8>>;
    fn write_chunk(&self, handle_len: usize) -> usize;
    fn read_chunk(&self) -> u32;
    fn open_handles(&self) -> Option<u64>;
}

/// Files held open at once when the server names no limit: a typical `sshd`
/// child allows 1024 open files, shared by at most `MaxSessions` (10) pool
/// channels, so about 100 each; 64 stays below that.
pub(super) const UNKNOWN_OPEN_HANDLES: usize = 64;
/// Files per packet at most: `BATCH_MAX_FILES` of every packet protocol
/// (`share/core/batch_wire.rs`).
pub(super) const FILES_CEILING: usize = 256;
/// Bytes per packet: the memory reserved for it and the work repeated when
/// it is lost, about 250 ms at the measured rate (as the Share packets).
pub(super) const BATCH_BYTES: u64 = 16 * 1024 * 1024;

/// The packet limits for a server that allows `known_open_handles` files
/// open per channel (`None`: it named none).
pub(super) fn limits(known_open_handles: Option<u64>) -> BatchLimits {
    let max_files = known_open_handles
        .map(|handles| {
            usize::try_from(handles)
                .unwrap_or(usize::MAX)
                .min(FILES_CEILING)
        })
        .filter(|files| *files >= 1)
        .unwrap_or(UNKNOWN_OPEN_HANDLES);
    BatchLimits {
        max_files,
        max_bytes: BATCH_BYTES,
    }
}

/// The packet limits of `backend`, or `None` while no pool channel can serve
/// (the files then take the main session one by one, as before). A first
/// channel is opened here when none exists, so the server's open-handle
/// limit is known before the first packet.
pub(super) fn advertised(backend: &SftpBackend) -> Option<BatchLimits> {
    if !backend.pool.available(backend) {
        return None;
    }
    let known = match backend.pool.known_open_handles() {
        Some(handles) => Some(handles),
        None => {
            let lease = backend.pool.lease(backend).ok()??;
            lease.channel().open_handles()
        }
    };
    Some(limits(known))
}

/// The channel of a packet, or "unsupported": the caller then moves the
/// files one by one.
pub(super) fn channel_or_unsupported<T>(lease: Option<T>) -> io::Result<T> {
    lease.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "Kein SFTP-Übertragungskanal für Pakete verfügbar",
        )
    })
}

/// `<destination>.se-upload-<16 hex>`: a private name nobody predicts.
pub(super) fn stage_name(destination: &str) -> String {
    let (parent, name) = match destination.rfind('/') {
        Some(index) => destination.split_at(index + 1),
        None => ("", destination),
    };
    let tail = format!(".se-upload-{}", crate::transfer::engine::random_hex());
    format!("{parent}{}", crate::vfs::fit_stage_name("", name, &tail))
}

/// The requests of one leased pool channel.
pub(super) struct ChannelWire {
    channel: Arc<PoolChannel>,
    connection: Arc<SftpConnection>,
}

impl ChannelWire {
    pub(super) fn new(channel: Arc<PoolChannel>, connection: Arc<SftpConnection>) -> Self {
        Self {
            channel,
            connection,
        }
    }

    /// A request failed: the connection learns of it; an OPEN that the
    /// server's open-file limit refuses is congestion (the flow backs off).
    fn failed(&self, error: SftpError, opening: bool) -> WireError {
        note_failure(&self.connection, &self.channel, &error);
        let answered = matches!(error, SftpError::Status(_) | SftpError::Limited(_));
        let error = if opening {
            open_error(error)
        } else {
            io_err(error)
        };
        WireError { error, answered }
    }
}

/// An OPEN that failed; "Too many open files" is the server's own refusal.
fn open_error(error: SftpError) -> io::Error {
    let refused = matches!(
        &error,
        SftpError::Status(status)
            if status.error_message.to_ascii_lowercase().contains("too many open files")
    );
    if !refused {
        return open_failed(error);
    }
    crate::vfs::congestion_error(
        format!("Der SFTP-Server hält keine weiteren Dateien offen ({error})"),
        None,
    )
}

impl BatchWire for ChannelWire {
    fn open_new(&self, path: &str) -> WireFuture<'_, String> {
        let path = path.to_string();
        Box::pin(async move {
            let attrs = FileAttributes::empty();
            let opened = self
                .channel
                .session
                .open(path, create_new_flags(), attrs)
                .await;
            opened
                .map(|handle| handle.handle)
                .map_err(|error| self.failed(error, true))
        })
    }

    fn write(&self, handle: &str, offset: u64, data: Vec<u8>) -> WireFuture<'_, ()> {
        let handle = handle.to_string();
        Box::pin(async move {
            let written = self.channel.session.write(handle, offset, data).await;
            written
                .map(|_| ())
                .map_err(|error| self.failed(error, false))
        })
    }

    fn close(&self, handle: &str) -> WireFuture<'_, ()> {
        let handle = handle.to_string();
        Box::pin(async move {
            let closed = self.channel.session.close(handle).await;
            closed
                .map(|_| ())
                .map_err(|error| self.failed(error, false))
        })
    }

    fn rename(&self, from: &str, to: &str) -> WireFuture<'_, ()> {
        let (from, to) = (from.to_string(), to.to_string());
        Box::pin(async move {
            let renamed = self.channel.session.rename(from, to).await;
            renamed
                .map(|_| ())
                .map_err(|error| self.failed(error, false))
        })
    }

    fn remove(&self, path: &str) -> WireFuture<'_, ()> {
        let path = path.to_string();
        Box::pin(async move {
            let removed = self.channel.session.remove(path).await;
            removed
                .map(|_| ())
                .map_err(|error| self.failed(error, false))
        })
    }

    fn exists(&self, path: &str) -> WireFuture<'_, bool> {
        let path = path.to_string();
        Box::pin(async move {
            match self.channel.session.lstat(path).await {
                Ok(_) => Ok(true),
                Err(SftpError::Status(status)) if status.status_code == StatusCode::NoSuchFile => {
                    Ok(false)
                }
                Err(error) => Err(self.failed(error, false)),
            }
        })
    }

    fn open_read(&self, path: &str) -> WireFuture<'_, String> {
        let path = path.to_string();
        Box::pin(async move {
            let attrs = FileAttributes::empty();
            let opened = self
                .channel
                .session
                .open(path, OpenFlags::READ, attrs)
                .await;
            opened
                .map(|handle| handle.handle)
                .map_err(|error| self.failed(error, true))
        })
    }

    fn read(&self, handle: &str, offset: u64, len: u32) -> WireFuture<'_, Vec<u8>> {
        let handle = handle.to_string();
        Box::pin(async move {
            match self.channel.session.read(handle, offset, len).await {
                Ok(data) => Ok(data.data),
                Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => {
                    Ok(Vec::new())
                }
                Err(error) => Err(self.failed(error, false)),
            }
        })
    }

    fn write_chunk(&self, handle_len: usize) -> usize {
        self.channel.write_chunk(handle_len)
    }

    fn read_chunk(&self) -> u32 {
        self.channel.read_chunk()
    }

    fn open_handles(&self) -> Option<u64> {
        self.channel.open_handles()
    }
}

/// Members in flight at once: as many as the server lets one channel hold
/// open, never more than the packet has.
pub(super) fn window(wire: &dyn BatchWire, members: usize) -> usize {
    let allowed = wire.open_handles().map_or(UNKNOWN_OPEN_HANDLES, |handles| {
        usize::try_from(handles).unwrap_or(usize::MAX)
    });
    members.min(allowed).max(1)
}
