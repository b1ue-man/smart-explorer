//! The record layer of the direct TCP picture link (`video_tcp_link.rs`):
//! the TLS handshake on the TCP stream, then records of a `u32` big-endian
//! length and that many bytes, and the token exchange of `video_tcp_tls.rs`.
//!
//! The reader and the writer share the rustls state behind a mutex held only
//! for synchronous rustls calls; each owns its half of the TCP stream and
//! waits for readiness on it, so the host's picture writer and a reader in
//! another task never wait for each other.
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rustls::Connection;
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::time::{timeout_at, Instant};

use super::tls::tls_io;
use crate::screen::core::video_tcp_wire::{TcpToken, CONFIRM, MAX_LINK_RECORD_BYTES, TOKEN_BYTES};

/// Why a TLS stage of one connection ended.
#[derive(Debug)]
pub(super) enum LinkError {
    Io(io::Error),
    Tls(rustls::Error),
    /// The peer closed before the expected bytes.
    Closed,
    Timeout,
}

struct ReadFn<F>(F);

impl<F: FnMut(&mut [u8]) -> io::Result<usize>> Read for ReadFn<F> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (self.0)(buf)
    }
}

struct WriteFn<F>(F);

impl<F: FnMut(&[u8]) -> io::Result<usize>> Write for WriteFn<F> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (self.0)(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn would_block(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock
}

fn lock(shared: &Mutex<Connection>) -> MutexGuard<'_, Connection> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

async fn ready<F: std::future::Future<Output = io::Result<()>>>(
    deadline: Instant,
    readiness: F,
) -> Result<(), LinkError> {
    timeout_at(deadline, readiness)
        .await
        .map_err(|_| LinkError::Timeout)?
        .map_err(LinkError::Io)
}

/// A connection after its handshake, split into its two directions.
pub(super) struct TlsLink {
    pub(super) local: SocketAddr,
    pub(super) remote: SocketAddr,
    pub(super) reader: TcpRecordReader,
    pub(super) writer: TcpRecordWriter,
}

/// Runs the TLS handshake of `conn` on `stream` until `deadline`.
pub(super) async fn handshake(
    stream: TcpStream,
    mut conn: Connection,
    deadline: Instant,
) -> Result<TlsLink, LinkError> {
    let local = stream.local_addr().map_err(LinkError::Io)?;
    let remote = stream.peer_addr().map_err(LinkError::Io)?;
    loop {
        while conn.wants_write() {
            match conn.write_tls(&mut WriteFn(|buf: &[u8]| stream.try_write(buf))) {
                Ok(_) => {}
                Err(error) if would_block(&error) => ready(deadline, stream.writable()).await?,
                Err(error) => return Err(LinkError::Io(error)),
            }
        }
        if !conn.is_handshaking() {
            break;
        }
        match conn.read_tls(&mut ReadFn(|buf: &mut [u8]| stream.try_read(buf))) {
            Ok(0) => return Err(LinkError::Closed),
            Ok(_) => {
                if let Err(error) = conn.process_new_packets() {
                    // Best effort: the alert rustls queued tells the peer why.
                    let _ = conn.write_tls(&mut WriteFn(|buf: &[u8]| stream.try_write(buf)));
                    return Err(LinkError::Tls(error));
                }
            }
            Err(error) if would_block(&error) => ready(deadline, stream.readable()).await?,
            Err(error) => return Err(LinkError::Io(error)),
        }
    }
    let (read, write) = stream.into_split();
    let shared = Arc::new(Mutex::new(conn));
    Ok(TlsLink {
        local,
        remote,
        reader: TcpRecordReader {
            shared: shared.clone(),
            read,
            max_record: MAX_LINK_RECORD_BYTES,
            received: 0,
        },
        writer: TcpRecordWriter {
            shared,
            write,
            sent: 0,
        },
    })
}

enum Pump {
    Full,
    /// The peer ended the TLS session (`close_notify`).
    Closed,
    Wait,
}

/// The reading direction of a link: records of a `u32` big-endian length
/// and that many bytes, each at most the reader's bound.
pub struct TcpRecordReader {
    shared: Arc<Mutex<Connection>>,
    read: OwnedReadHalf,
    max_record: usize,
    received: u64,
}

impl TcpRecordReader {
    /// The next record; `None` when the peer ended the link cleanly between
    /// two records. A length above the bound is refused before any memory
    /// for it is requested.
    pub async fn recv(&mut self) -> io::Result<Option<Vec<u8>>> {
        let mut head = [0u8; 4];
        let mut filled = 0;
        if !self.fill(&mut head, &mut filled).await? {
            return match filled {
                0 => Ok(None),
                _ => Err(io::ErrorKind::UnexpectedEof.into()),
            };
        }
        let length = u32::from_be_bytes(head) as usize;
        if length > self.max_record {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "TCP picture record exceeds its limit",
            ));
        }
        let mut record = vec![0u8; length];
        let mut filled = 0;
        if self.fill(&mut record, &mut filled).await? {
            Ok(Some(record))
        } else {
            Err(io::ErrorKind::UnexpectedEof.into())
        }
    }

    /// Lowers the largest record this reader takes (at most
    /// `MAX_LINK_RECORD_BYTES`), e.g. to the negotiated frame contract.
    pub fn set_max_record(&mut self, bytes: usize) {
        self.max_record = bytes.min(MAX_LINK_RECORD_BYTES);
    }

    /// Plaintext bytes received so far (records with their heads).
    pub fn bytes_received(&self) -> u64 {
        self.received
    }

    /// Fills `buf`; `false` when the peer ended the session first.
    pub(super) async fn fill(&mut self, buf: &mut [u8], filled: &mut usize) -> io::Result<bool> {
        loop {
            match self.pump(buf, filled)? {
                Pump::Full => return Ok(true),
                Pump::Closed => return Ok(false),
                Pump::Wait => self.read.readable().await?,
            }
        }
    }

    fn pump(&mut self, buf: &mut [u8], filled: &mut usize) -> io::Result<Pump> {
        let shared = self.shared.clone();
        let mut conn = lock(&shared);
        let read = &self.read;
        let mut eof = false;
        loop {
            while *filled < buf.len() {
                match conn.reader().read(&mut buf[*filled..]) {
                    Ok(0) => return Ok(Pump::Closed),
                    Ok(count) => {
                        *filled += count;
                        self.received += count as u64;
                    }
                    Err(error) if would_block(&error) && eof => {
                        return Err(io::ErrorKind::UnexpectedEof.into())
                    }
                    Err(error) if would_block(&error) => break,
                    Err(error) => return Err(error),
                }
            }
            if *filled == buf.len() {
                return Ok(Pump::Full);
            }
            match conn.read_tls(&mut ReadFn(|out: &mut [u8]| read.try_read(out))) {
                // The reader now reports `close_notify` (Ok(0)) or an
                // unclean end (UnexpectedEof).
                Ok(0) => eof = true,
                Ok(_) => {
                    conn.process_new_packets().map_err(tls_io)?;
                }
                Err(error) if would_block(&error) => return Ok(Pump::Wait),
                Err(error) => return Err(error),
            }
        }
    }
}

/// The writing direction of a link.
pub struct TcpRecordWriter {
    shared: Arc<Mutex<Connection>>,
    write: OwnedWriteHalf,
    sent: u64,
}

impl TcpRecordWriter {
    /// Writes one record: the `u32` length of all `parts` together, then
    /// the parts as they are (a picture is not copied into one buffer
    /// first). Returns when the kernel took the last byte.
    pub async fn send(&mut self, parts: &[&[u8]]) -> io::Result<()> {
        let total: usize = parts.iter().map(|part| part.len()).sum();
        if total > MAX_LINK_RECORD_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "TCP picture record exceeds its limit",
            ));
        }
        let head = (total as u32).to_be_bytes();
        let mut chunks: Vec<&[u8]> = Vec::with_capacity(parts.len() + 1);
        chunks.push(&head);
        chunks.extend_from_slice(parts);
        self.write_all(&chunks).await
    }

    /// Plaintext bytes written so far (records with their heads).
    pub fn bytes_sent(&self) -> u64 {
        self.sent
    }

    /// Ends the link cleanly: `close_notify`, then the TCP write side.
    pub async fn finish(mut self) -> io::Result<()> {
        lock(&self.shared).send_close_notify();
        self.write_all(&[]).await?;
        self.write.shutdown().await
    }

    pub(super) async fn write_all(&mut self, chunks: &[&[u8]]) -> io::Result<()> {
        let (mut index, mut offset) = (0, 0);
        loop {
            let (done, blocked) = self.push(chunks, &mut index, &mut offset)?;
            if done {
                return Ok(());
            }
            if blocked {
                self.write.writable().await?;
            }
        }
    }

    fn push(
        &mut self,
        chunks: &[&[u8]],
        index: &mut usize,
        offset: &mut usize,
    ) -> io::Result<(bool, bool)> {
        let shared = self.shared.clone();
        let mut conn = lock(&shared);
        while *index < chunks.len() {
            let rest = &chunks[*index][*offset..];
            if rest.is_empty() {
                *index += 1;
                *offset = 0;
                continue;
            }
            let taken = conn.writer().write(rest)?;
            if taken == 0 {
                break;
            }
            *offset += taken;
            self.sent += taken as u64;
        }
        let write = &self.write;
        let mut blocked = false;
        while conn.wants_write() {
            match conn.write_tls(&mut WriteFn(|buf: &[u8]| write.try_write(buf))) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(_) => {}
                Err(error) if would_block(&error) => {
                    blocked = true;
                    break;
                }
                Err(error) => return Err(error),
            }
        }
        Ok((*index == chunks.len() && !conn.wants_write(), blocked))
    }
}

async fn bounded<T>(
    deadline: Instant,
    work: impl std::future::Future<Output = io::Result<T>>,
) -> Result<T, LinkError> {
    timeout_at(deadline, work)
        .await
        .map_err(|_| LinkError::Timeout)?
        .map_err(LinkError::Io)
}

/// Host: the 32 token bytes the viewer wrote first.
pub(super) async fn read_token(
    reader: &mut TcpRecordReader,
    deadline: Instant,
) -> Result<[u8; TOKEN_BYTES], LinkError> {
    let mut token = [0u8; TOKEN_BYTES];
    let mut filled = 0;
    let complete = bounded(deadline, reader.fill(&mut token, &mut filled)).await?;
    if complete {
        Ok(token)
    } else {
        Err(LinkError::Closed)
    }
}

/// Host: confirms the token.
pub(super) async fn confirm(
    writer: &mut TcpRecordWriter,
    deadline: Instant,
) -> Result<(), LinkError> {
    bounded(deadline, writer.write_all(&[&CONFIRM])).await
}

/// Viewer: writes the token and waits for the host's confirmation.
pub(super) async fn present_token(
    link: &mut TlsLink,
    token: &TcpToken,
    deadline: Instant,
) -> Result<(), LinkError> {
    bounded(deadline, link.writer.write_all(&[&token.0])).await?;
    let mut answer = [0u8; CONFIRM.len()];
    let mut filled = 0;
    let complete = bounded(deadline, link.reader.fill(&mut answer, &mut filled)).await?;
    if !complete || answer != CONFIRM {
        return Err(LinkError::Closed);
    }
    Ok(())
}
