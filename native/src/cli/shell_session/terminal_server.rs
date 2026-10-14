//! Serves one connection of a hosted terminal session: the frames of a
//! client's terminal in, the remote terminal's bytes, notices and the end out.
//! The client was admitted by the attach service (`server.rs`) with hello
//! version 2. A frame that does not belong to a terminal session (`SUBMIT`,
//! `INPUT`, anything unknown) is a protocol error and ends the connection.
use super::engine::OUTBOUND_CAPACITY;
use super::server::{deny, ATTACHED_WRITE_TIMEOUT};
use super::terminal_engine::TerminalEngine;
use super::types::{Role, TerminalEvent, TerminalSize};
use super::wire;
use crossbeam_channel::Receiver;
use std::io;
use std::net::{Shutdown, TcpStream};
use std::sync::Arc;

/// Serves one admitted client until it leaves or its connection fails. The
/// owner leaving ends the session; an attached client detaches.
pub(super) fn connection(
    engine: Arc<TerminalEngine>,
    role: Role,
    label: &str,
    stream: TcpStream,
) -> io::Result<()> {
    let (sender, outbound) = crossbeam_channel::bounded(OUTBOUND_CAPACITY);
    let id = match engine.join(role, label, sender) {
        Ok(id) => id,
        Err(message) => {
            let _ = deny::<()>(&stream, &message);
            let _ = stream.shutdown(Shutdown::Both);
            return Ok(());
        }
    };
    let result = serve_admitted(&engine, id, role, label, &stream, outbound);
    match role {
        Role::Owner => engine.close(),
        Role::Attached => engine.leave(id),
    }
    let _ = stream.shutdown(Shutdown::Read);
    result
}

fn serve_admitted(
    engine: &Arc<TerminalEngine>,
    id: u64,
    role: Role,
    label: &str,
    stream: &TcpStream,
    outbound: Receiver<TerminalEvent>,
) -> io::Result<()> {
    let mut writer = stream.try_clone()?;
    writer.set_write_timeout(match role {
        Role::Owner => None,
        Role::Attached => Some(ATTACHED_WRITE_TIMEOUT),
    })?;
    wire::write_json(&mut writer, wire::WELCOME, &engine.info(role))?;
    std::thread::Builder::new()
        .name("se-term-forward".into())
        .spawn(move || forward(writer, outbound))?;
    let mut reader = stream;
    loop {
        let (tag, payload) = wire::read_frame(&mut reader)?;
        match tag {
            wire::TERM_INPUT => {
                // A client that is no longer part of the session is done.
                if engine.input(id, payload).is_err() {
                    return Ok(());
                }
            }
            wire::TERM_RESIZE => {
                let size: TerminalSize = wire::decode(&payload)?;
                // A terminal that reports no usable size keeps its last one.
                if size.validate().is_ok() && engine.resize(id, size).is_err() {
                    return Ok(());
                }
            }
            wire::CANCEL if payload.is_empty() => engine.cancel(label),
            wire::LEAVE if payload.is_empty() => return Ok(()),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected attach frame",
                ))
            }
        }
    }
}

/// Writes the session's events to the client until the session drops this
/// participant (its queue closes) or the connection fails.
fn forward(mut writer: TcpStream, outbound: Receiver<TerminalEvent>) {
    for event in outbound {
        if write_event(&mut writer, event).is_err() {
            break;
        }
    }
    let _ = writer.shutdown(Shutdown::Both);
}

fn write_event(writer: &mut TcpStream, event: TerminalEvent) -> io::Result<()> {
    match event {
        TerminalEvent::Output(bytes) => {
            for chunk in bytes.chunks(wire::MAX_DATA) {
                wire::write_frame(writer, wire::TERM_OUTPUT, chunk)?;
            }
            Ok(())
        }
        TerminalEvent::Notice(text) => wire::write_json(writer, wire::NOTICE, &wire::Text { text }),
        TerminalEvent::Ended { exit_code, message } => {
            wire::write_json(writer, wire::ENDED, &wire::Ended { exit_code, message })
        }
    }
}
