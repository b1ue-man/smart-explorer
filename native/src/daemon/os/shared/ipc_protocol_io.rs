//! Bounded IPC transport, separated from the request/reply model.
use std::io::{self, Write};
use std::net::TcpStream;
use std::time::Duration;
use super::{IpcRequest, IpcResponse};
use super::super::line::{read_line_limited_from_stream, MAX_IPC_LINE};

pub(in crate::daemon) fn write_response(stream: &mut TcpStream, response: &IpcResponse) -> io::Result<()> {
    let mut line = serde_json::to_string(response).map_err(eio)?;
    line.push('\n');
    if line.len() > MAX_IPC_LINE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ipc response exceeds line budget",
        ));
    }
    stream.write_all(line.as_bytes())?;
    stream.flush()
}

pub(in crate::daemon) fn set_stream_timeout(stream: &TcpStream, timeout: Option<Duration>) {
    let _ = stream.set_read_timeout(timeout);
    let _ = stream.set_write_timeout(timeout);
}

pub(in crate::daemon) fn write_request(stream: &mut TcpStream, req: &IpcRequest) -> io::Result<()> {
    let mut line = super::super::runtime_hello::encode_request(req)?;
    if line.len()>=MAX_IPC_LINE {return Err(io::Error::new(io::ErrorKind::InvalidInput,"ipc request exceeds line budget"));}
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    stream.flush()
}

pub(in crate::daemon) fn read_response(stream: &mut TcpStream) -> io::Result<IpcResponse> {
    let mut line = String::new();
    read_line_limited_from_stream(stream, &mut line, MAX_IPC_LINE)?;
    serde_json::from_str(line.trim()).map_err(eio)
}

fn eio<E: std::fmt::Display>(error: E) -> io::Error {
    io::Error::other(error.to_string())
}
