use super::backend::AgentBackend;
use super::mux::Mux;
use crate::agent_proto::Frame;
use crossbeam_channel::Receiver;
use std::io::{self, Read, Write};
use std::sync::Arc;

/// `std::io::Read` over a streamed `Read` op.
struct AgentReadStream {
    mux: Arc<Mux>,
    id: u64,
    rx: Receiver<Frame>,
    buf: Vec<u8>,
    pos: usize,
    done: bool,
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
                Ok(Frame::Data(d)) => {
                    self.buf = d;
                    self.pos = 0;
                }
                Ok(Frame::End) => {
                    self.done = true;
                    return Ok(0);
                }
                Ok(Frame::Err(e)) => return Err(io::Error::other(e)),
                Ok(_) => continue,
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
    mux: Arc<Mux>,
    id: u64,
    rx: Receiver<Frame>,
    finished: bool,
}

impl AgentWriteStream {
    fn finish(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        self.mux.send(self.id, Frame::End)?;
        let r = match self.rx.recv() {
            Ok(Frame::Ok) => Ok(()),
            Ok(Frame::Err(e)) => Err(io::Error::other(e)),
            Ok(_) => Err(io::Error::other("unexpected agent reply to write")),
            Err(_) => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "agent write stream closed",
            )),
        };
        self.mux.unregister(self.id);
        r
    }
}

impl Write for AgentWriteStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.finished {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "agent write stream already closed",
            ));
        }
        self.mux.send(self.id, Frame::Data(buf.to_vec()))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.finish()
    }
}

impl Drop for AgentWriteStream {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

impl AgentBackend {
    /// Begin a streamed read of `path`. Blocks for the first frame so an open
    /// error falls back to `inner` synchronously.
    pub(super) fn agent_open_read(&self, path: &str) -> Option<Box<dyn Read + Send>> {
        let (id, rx) = self.mux.register();
        if self
            .mux
            .send(
                id,
                Frame::Read {
                    path: path.to_string(),
                    offset: 0,
                    len: 0,
                },
            )
            .is_err()
        {
            self.mux.unregister(id);
            return None;
        }
        match rx.recv() {
            Ok(Frame::Data(d)) => Some(Box::new(AgentReadStream {
                mux: self.mux.clone(),
                id,
                rx,
                buf: d,
                pos: 0,
                done: false,
            })),
            Ok(Frame::End) => Some(Box::new(AgentReadStream {
                mux: self.mux.clone(),
                id,
                rx,
                buf: Vec::new(),
                pos: 0,
                done: true,
            })),
            _ => {
                self.mux.unregister(id);
                None
            }
        }
    }

    /// Begin a streamed write of `path`. Blocks for the agent's ready-ack so a
    /// path/permission error falls back to `inner` synchronously.
    pub(super) fn agent_open_write(&self, path: &str) -> Option<Box<dyn Write + Send>> {
        let (id, rx) = self.mux.register();
        if self.mux.send(id, Frame::Write(path.to_string())).is_err() {
            self.mux.unregister(id);
            return None;
        }
        match rx.recv() {
            Ok(Frame::Progress { .. }) => Some(Box::new(AgentWriteStream {
                mux: self.mux.clone(),
                id,
                rx,
                finished: false,
            })),
            _ => {
                self.mux.unregister(id);
                None
            }
        }
    }
}
