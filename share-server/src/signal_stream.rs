//! Socket options and I/O shared by ws and wss without splitting TLS state.

use crate::websocket_socket::SocketIo;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;

use rustls::{ServerConfig, ServerConnection, StreamOwned};

pub(super) enum SignalStream {
    Plain(SocketIo),
    Tls(Box<StreamOwned<ServerConnection, SocketIo>>),
}

impl SignalStream {
    pub(super) fn tls(stream: TcpStream, config: Arc<ServerConfig>) -> io::Result<Self> {
        let connection = ServerConnection::new(config).map_err(io::Error::other)?;
        Ok(Self::Tls(Box::new(StreamOwned::new(
            connection,
            SocketIo::new(stream)?,
        ))))
    }

    pub(super) fn socket(&self) -> &TcpStream {
        match self {
            Self::Plain(socket) => socket.socket(),
            Self::Tls(tls) => tls.sock.socket(),
        }
    }

    pub(super) fn plain(stream: TcpStream) -> io::Result<Self> {
        Ok(Self::Plain(SocketIo::new(stream)?))
    }

    pub(super) fn set_write_deadline(&mut self, deadline: Option<Instant>) {
        match self {
            Self::Plain(io) => io.set_write_deadline(deadline),
            Self::Tls(tls) => tls.sock.set_write_deadline(deadline),
        }
    }

    pub(super) fn notify(&self) -> Arc<Notify> {
        match self {
            Self::Plain(io) => io.notify(),
            Self::Tls(tls) => tls.sock.notify(),
        }
    }

    pub(super) fn wait_readable(&self, deadline: Instant) -> io::Result<()> {
        match self {
            Self::Plain(io) => io.wait_readable(deadline),
            Self::Tls(tls) => tls.sock.wait_readable(deadline),
        }
    }

    pub(super) fn set_read_timeout(&self, value: Option<Duration>) -> io::Result<()> {
        self.socket().set_read_timeout(value)
    }
}

impl Read for SignalStream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(socket) => socket.read(bytes),
            Self::Tls(tls) => tls.read(bytes),
        }
    }
}

impl Write for SignalStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(socket) => socket.write(bytes),
            Self::Tls(tls) => tls.write(bytes),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(socket) => socket.flush(),
            Self::Tls(tls) => tls.flush(),
        }
    }
}
