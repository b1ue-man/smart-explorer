use super::backend::AgentBackend;
use super::mux::Mux;
use super::pool::ChannelLease;
use super::route::RequestRx;
use crate::agent_proto::{unpack, BlockSender, Frame, CHUNK};
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::Instant;

/// `std::io::Read` over a streamed `Read` op.
struct AgentReadStream {
    mux: Arc<Mux>,
    id: u64,
    rx: RequestRx,
    buf: Vec<u8>,
    pos: usize,
    done: bool,
    /// The server may answer with `DataLz4` (this read was `ReadLz4`).
    lz4: bool,
    /// Keeps the pooled channel counted as busy while the stream is open.
    _lease: ChannelLease,
}

impl Read for AgentReadStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.pos < self.buf.len() {
                let n = (self.buf.len() - self.pos).min(out.len());
                out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
                self.pos += n;
                return Ok(n);
            }
            if self.done {
                return Ok(0);
            }
            match self.rx.recv() {
                Ok(Frame::Data(d)) if d.len() <= CHUNK => {
                    self.buf = d;
                    self.pos = 0;
                }
                Ok(Frame::DataLz4(block)) if self.lz4 => {
                    self.buf = unpack(&block)?;
                    self.pos = 0;
                }
                Ok(Frame::Data(_)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "agent read frame exceeds the protocol chunk limit",
                    ));
                }
                Ok(Frame::End) => {
                    self.done = true;
                    return Ok(0);
                }
                Ok(Frame::Err(e)) => return Err(super::agent_error::agent_error(e)),
                Ok(other) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unexpected agent read-stream reply: {other:?}"),
                    ));
                }
                Err(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "agent read stream closed",
                    ))
                }
            }
        }
    }
}

impl Drop for AgentReadStream {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.mux.send(self.id, Frame::Cancel);
        }
        self.mux.unregister(self.id);
    }
}

/// `std::io::Write` over a streamed `Write` op.
struct AgentWriteStream {
    /// Shared by the ranges of one stage, which all run on its channel.
    lease: Arc<ChannelLease>,
    mux: Arc<Mux>,
    id: u64,
    rx: RequestRx,
    state: WriteState,
    /// Packs the blocks when the server announced `lz4-v1`.
    lz4: Option<BlockSender>,
    /// Bytes this stream must carry exactly (a range of a stage).
    expected: Option<u64>,
    sent: u64,
}

enum WriteState {
    Open,
    Committed,
    Failed {
        kind: io::ErrorKind,
        message: String,
    },
}

impl AgentWriteStream {
    fn finish(&mut self) -> io::Result<()> {
        match &self.state {
            WriteState::Committed => return Ok(()),
            WriteState::Failed { kind, message } => {
                return Err(io::Error::new(*kind, message.clone()))
            }
            WriteState::Open => {}
        }
        if let Some(expected) = self.expected.filter(|expected| *expected != self.sent) {
            // A range that is short never reaches the server's End.
            let error = io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} statt {expected} Bytes für den Bereich", self.sent),
            );
            let _ = self.mux.send(self.id, Frame::Cancel);
            self.mux.unregister(self.id);
            self.remember_failure(&error);
            return Err(error);
        }
        if let Err(error) = self.mux.send(self.id, Frame::End) {
            self.mux.unregister(self.id);
            self.remember_failure(&error);
            return Err(error);
        }
        let result = match self.rx.recv() {
            Ok(Frame::Ok) => Ok(()),
            Ok(Frame::Err(e)) => Err(super::agent_error::agent_error(e)),
            Ok(other) => {
                self.lease.invalidate(&self.mux);
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unexpected agent write-stream reply: {other:?}"),
                ))
            }
            Err(_) => {
                self.lease.invalidate(&self.mux);
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "agent write stream closed",
                ))
            }
        };
        self.mux.unregister(self.id);
        match &result {
            Ok(()) => self.state = WriteState::Committed,
            Err(error) => self.remember_failure(error),
        }
        result
    }

    fn remember_failure(&mut self, error: &io::Error) {
        self.state = WriteState::Failed {
            kind: error.kind(),
            message: error.to_string(),
        };
    }
}

impl Write for AgentWriteStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !matches!(self.state, WriteState::Open) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "agent write stream already closed",
            ));
        }
        let written = buf.len().min(CHUNK);
        if let Some(expected) = self.expected {
            if self.sent.saturating_add(written as u64) > expected {
                let error = io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Quelle liefert mehr als die angekündigten {expected} Bytes"),
                );
                let _ = self.mux.send(self.id, Frame::Cancel);
                self.mux.unregister(self.id);
                self.remember_failure(&error);
                return Err(error);
            }
        }
        let chunk = &buf[..written];
        let frame = match self
            .lz4
            .as_mut()
            .and_then(|sender| sender.try_pack(chunk, self.sent))
        {
            Some(block) => Frame::DataLz4(block),
            None => Frame::Data(chunk.to_vec()),
        };
        let started = Instant::now();
        if let Err(error) = self.mux.send(self.id, frame) {
            // The server may have ended the upload; its reply names the
            // reason. Either way the upload is over: never commit it later.
            let error = match self.rx.try_recv() {
                Ok(Frame::Err(message)) => super::agent_error::agent_error(message),
                _ => error,
            };
            let _ = self.mux.send(self.id, Frame::Cancel);
            self.mux.unregister(self.id);
            self.remember_failure(&error);
            return Err(error);
        }
        if let Some(sender) = self.lz4.as_mut() {
            sender.record_send(written, started.elapsed(), Instant::now());
        }
        self.sent += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.finish()
    }
}

impl Drop for AgentWriteStream {
    fn drop(&mut self) {
        if matches!(self.state, WriteState::Open) {
            // Dropping a writer is an abort, not an implicit commit. Wake the
            // server-side receiver after setting cancellation so it refuses
            // promotion; cleanup remains subject to stable ownership proof.
            let _ = self.mux.send(self.id, Frame::Cancel);
            let _ = self.mux.send(self.id, Frame::End);
            self.mux.unregister(self.id);
        }
    }
}

impl AgentBackend {
    /// Begin a streamed read of `path` from byte `offset`. Protocol-v6 makes
    /// this mandatory, so every transport, protocol, or remote failure is
    /// returned to the caller.
    pub(super) fn agent_open_read_at(
        &self,
        path: &str,
        offset: u64,
    ) -> io::Result<Box<dyn Read + Send>> {
        self.agent_open_read_len(path, offset, 0)
    }

    /// The same for `[offset, offset + len)`: the agent sends `len` bytes
    /// (fewer only at the end of the file); `len` 0 means to the end.
    pub(super) fn agent_open_read_len(
        &self,
        path: &str,
        offset: u64,
        len: u64,
    ) -> io::Result<Box<dyn Read + Send>> {
        let lease = self.pool.lease();
        let lz4 = self.features().lz4;
        let opened = lease.retry_safe(|mux| open_read_once(mux, path, (offset, len), lz4))?;
        let (buf, done) = match opened.first {
            Frame::Data(d) if d.len() <= CHUNK => (d, false),
            Frame::DataLz4(block) if lz4 => match unpack(&block) {
                Ok(raw) => (raw, false),
                Err(error) => {
                    lease.invalidate(&opened.mux);
                    opened.mux.unregister(opened.id);
                    return Err(error);
                }
            },
            Frame::End => (Vec::new(), true),
            Frame::Data(_) => {
                lease.invalidate(&opened.mux);
                opened.mux.unregister(opened.id);
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "agent read frame exceeds the protocol chunk limit",
                ));
            }
            Frame::Err(error) => {
                opened.mux.unregister(opened.id);
                return Err(super::agent_error::agent_error(error));
            }
            other => {
                lease.invalidate(&opened.mux);
                opened.mux.unregister(opened.id);
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unexpected agent reply to read: {other:?}"),
                ));
            }
        };
        Ok(Box::new(AgentReadStream {
            mux: opened.mux,
            id: opened.id,
            rx: opened.rx,
            buf,
            pos: 0,
            done,
            lz4,
            _lease: lease,
        }))
    }

    /// Begin a streamed write of `path`. Protocol-v6 makes this mandatory, so
    /// every transport, protocol, or remote failure is returned to the caller.
    pub(super) fn agent_open_write(&self, path: &str) -> io::Result<Box<dyn Write + Send>> {
        self.agent_open_write_request(Frame::Write(path.to_string()))
    }

    pub(super) fn agent_open_write_new(&self, path: &str) -> io::Result<Box<dyn Write + Send>> {
        self.agent_open_write_request(Frame::WriteNew(path.to_string()))
    }

    fn agent_open_write_request(&self, request: Frame) -> io::Result<Box<dyn Write + Send>> {
        let lease = Arc::new(self.pool.lease());
        let mux = lease.mutation_mux()?;
        open_write_on(lease, mux, request, None, self.features().lz4)
    }
}

/// Starts the upload `request` (`Write`, `WriteNew` or `WriteAt`) on `mux` and
/// waits for the server's `Progress`: from then on the returned writer
/// streams the bytes, as `DataLz4` blocks when `lz4` and they shrink. With
/// `expected` it also commits only after exactly that many bytes.
pub(super) fn open_write_on(
    lease: Arc<ChannelLease>,
    mux: Arc<Mux>,
    request: Frame,
    expected: Option<u64>,
    lz4: bool,
) -> io::Result<Box<dyn Write + Send>> {
    let (id, rx) = mux.register();
    if let Err(error) = mux.send(id, request) {
        mux.unregister(id);
        return Err(error);
    }
    let result = match rx.recv() {
        Ok(Frame::Progress { .. }) => {
            let lz4 = lz4.then(|| {
                let mut sender = BlockSender::new();
                sender.start_file();
                sender
            });
            return Ok(Box::new(AgentWriteStream {
                lease,
                mux,
                id,
                rx,
                state: WriteState::Open,
                lz4,
                expected,
                sent: 0,
            }));
        }
        Ok(Frame::Err(error)) => Err(super::agent_error::agent_error(error)),
        Ok(other) => {
            lease.invalidate(&mux);
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected agent reply to write: {other:?}"),
            ))
        }
        Err(_) => {
            lease.invalidate(&mux);
            Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "agent write stream closed before opening",
            ))
        }
    };
    mux.unregister(id);
    result
}

struct ReadOpening {
    mux: Arc<Mux>,
    id: u64,
    rx: RequestRx,
    first: Frame,
}

fn open_read_once(
    mux: &Arc<Mux>,
    path: &str,
    range: (u64, u64),
    lz4: bool,
) -> io::Result<ReadOpening> {
    let (id, rx) = mux.register();
    let (offset, len) = range;
    let path = path.to_string();
    // `ReadLz4` tells the server that this client unpacks `DataLz4`; it is
    // sent only to a server that announced `lz4-v1`.
    let request = if lz4 {
        Frame::ReadLz4 { path, offset, len }
    } else {
        Frame::Read { path, offset, len }
    };
    if let Err(error) = mux.send(id, request) {
        mux.unregister(id);
        return Err(error);
    }
    let first = rx.recv().map_err(|_| {
        mux.unregister(id);
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "agent read stream closed before opening",
        )
    })?;
    Ok(ReadOpening {
        mux: mux.clone(),
        id,
        rx,
        first,
    })
}

#[cfg(test)]
mod tests {
    use super::AgentBackend;
    use crate::vfs::Backend;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;

    fn agent_with_open_error(
        operation: &'static str,
        message: &'static str,
    ) -> (AgentBackend, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut reader = socket.try_clone().unwrap();
            let (hello_id, _) = crate::agent_proto::read_frame(&mut reader)
                .unwrap()
                .unwrap();
            crate::agent_proto::write_frame(
                &mut socket,
                hello_id,
                &crate::agent_proto::Frame::HelloOk {
                    proto: crate::agent_proto::PROTO_VERSION,
                    version: "test".into(),
                },
            )
            .unwrap();
            let (request_id, request) = crate::agent_proto::read_frame(&mut reader)
                .unwrap()
                .unwrap();
            assert!(match operation {
                "read" => matches!(request, crate::agent_proto::Frame::Read { .. }),
                "write" => matches!(request, crate::agent_proto::Frame::Write(_)),
                _ => false,
            });
            crate::agent_proto::write_frame(
                &mut socket,
                request_id,
                &crate::agent_proto::Frame::Err(message.into()),
            )
            .unwrap();
        });
        let client = TcpStream::connect(address).unwrap();
        let backend = AgentBackend::from_streams(
            Box::new(client.try_clone().unwrap()) as Box<dyn Read + Send>,
            Box::new(client) as Box<dyn Write + Send>,
            Arc::new(crate::vfs::LocalBackend::new("/")),
        )
        .unwrap();
        (backend, server)
    }

    #[test]
    fn open_read_and_write_surface_agent_errors_without_fallback() {
        let (reader, server) = agent_with_open_error("read", "remote read denied");
        let error = reader.open_read("/denied").err().unwrap();
        assert!(error.to_string().contains("remote read denied"));
        drop(reader);
        server.join().unwrap();

        let (writer, server) = agent_with_open_error("write", "remote write denied");
        let error = writer.open_write("/denied").err().unwrap();
        assert!(error.to_string().contains("remote write denied"));
        drop(writer);
        server.join().unwrap();
    }
}
