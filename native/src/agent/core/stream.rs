use super::backend::AgentBackend;
use super::mux::Mux;
use super::pool::ChannelLease;
use super::route::RequestRx;
use crate::agent_proto::{Frame, CHUNK};
use std::io::{self, Read, Write};
use std::sync::Arc;

/// `std::io::Read` over a streamed `Read` op.
struct AgentReadStream {
    mux: Arc<Mux>,
    id: u64,
    rx: RequestRx,
    buf: Vec<u8>,
    pos: usize,
    done: bool,
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
    lease: ChannelLease,
    mux: Arc<Mux>,
    id: u64,
    rx: RequestRx,
    state: WriteState,
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
        if let Err(error) = self.mux.send(self.id, Frame::Data(buf[..written].to_vec())) {
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
        let lease = self.pool.lease();
        let opened = lease.retry_safe(|mux| open_read_once(mux, path, offset))?;
        let (buf, done) = match opened.first {
            Frame::Data(d) if d.len() <= CHUNK => (d, false),
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
        let lease = self.pool.lease();
        let mux = lease.mutation_mux()?;
        let (id, rx) = mux.register();
        if let Err(error) = mux.send(id, request) {
            mux.unregister(id);
            return Err(error);
        }
        let result = match rx.recv() {
            Ok(Frame::Progress { .. }) => {
                return Ok(Box::new(AgentWriteStream {
                    lease,
                    mux,
                    id,
                    rx,
                    state: WriteState::Open,
                }))
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
}

struct ReadOpening {
    mux: Arc<Mux>,
    id: u64,
    rx: RequestRx,
    first: Frame,
}

fn open_read_once(mux: &Arc<Mux>, path: &str, offset: u64) -> io::Result<ReadOpening> {
    let (id, rx) = mux.register();
    if let Err(error) = mux.send(
        id,
        Frame::Read {
            path: path.to_string(),
            offset,
            len: 0,
        },
    ) {
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
