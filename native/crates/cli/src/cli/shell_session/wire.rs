//! Frames of the attach protocol between a session's host and its clients
//! (the owner's own front-end included): one tag byte, a 32-bit big-endian
//! length, then the payload. Control payloads are JSON, output payloads raw
//! bytes (own command output with its 8-byte request tag in front).
//!
//! Version 1 carries command sessions. Version 2 adds terminal sessions: the
//! client sends its terminal's input as raw bytes (`TERM_INPUT`) and its size
//! as JSON (`TERM_RESIZE`), the host sends the remote terminal's output as raw
//! bytes (`TERM_OUTPUT`) next to the unchanged `NOTICE` and `ENDED`. A host
//! accepts version 2 for every session and version 1 only for command
//! sessions; clients send version 1 to command sessions so that owners of
//! older releases stay reachable.
use super::types::{CommandOutcome, CommandRequest};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io::{self, Read, Write};

/// Hello version of a client of a command session.
pub(super) const PROTOCOL_VERSION_COMMANDS: u32 = 1;
/// Hello version of a client of a terminal session.
pub(super) const PROTOCOL_VERSION_TERMINAL: u32 = 2;
/// Output and input chunk; the Exec stream's data frame size.
pub(super) const MAX_DATA: usize = 64 * 1024;
/// Hello: ids, a token and a label.
const MAX_HELLO: usize = 4 * 1024;
/// One submitted request: up to 1 MiB of command and input text, which JSON
/// escaping can grow up to six times.
const MAX_SUBMIT: usize = 8 * 1024 * 1024;
const MAX_CONTROL: usize = 64 * 1024;

// Client to host.
pub(super) const HELLO: u8 = 1;
pub(super) const SUBMIT: u8 = 2;
pub(super) const INPUT: u8 = 3;
pub(super) const CANCEL: u8 = 4;
pub(super) const LEAVE: u8 = 5;
/// Terminal sessions: input bytes of the client's terminal.
pub(super) const TERM_INPUT: u8 = 6;
/// Terminal sessions: the client's terminal size, JSON `TerminalSize`.
pub(super) const TERM_RESIZE: u8 = 7;
// Host to client.
pub(super) const WELCOME: u8 = 16;
pub(super) const DENIED: u8 = 17;
pub(super) const REJECTED: u8 = 18;
pub(super) const OWN_STDOUT: u8 = 19;
pub(super) const OWN_STDERR: u8 = 20;
pub(super) const OWN_DONE: u8 = 21;
pub(super) const ACT_STARTED: u8 = 22;
pub(super) const ACT_STDOUT: u8 = 23;
pub(super) const ACT_STDERR: u8 = 24;
pub(super) const ACT_DONE: u8 = 25;
pub(super) const NOTICE: u8 = 26;
pub(super) const ENDED: u8 = 27;
/// Terminal sessions: output bytes of the remote terminal.
pub(super) const TERM_OUTPUT: u8 = 28;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Hello {
    pub(super) version: u32,
    pub(super) session: String,
    pub(super) token: String,
    pub(super) label: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Submit {
    pub(super) tag: u64,
    pub(super) request: CommandRequest,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Rejected {
    pub(super) tag: u64,
    pub(super) message: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct OwnDone {
    pub(super) tag: u64,
    pub(super) outcome: CommandOutcome,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct ActStarted {
    pub(super) by: String,
    pub(super) command: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct ActDone {
    pub(super) by: String,
    pub(super) exit_code: Option<i32>,
    pub(super) cwd: Option<String>,
}

/// NOTICE and DENIED.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Text {
    pub(super) text: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Ended {
    pub(super) exit_code: Option<i32>,
    pub(super) message: String,
}

fn limit(tag: u8) -> usize {
    match tag {
        HELLO => MAX_HELLO,
        SUBMIT => MAX_SUBMIT,
        INPUT | ACT_STDOUT | ACT_STDERR | TERM_INPUT | TERM_OUTPUT => MAX_DATA,
        OWN_STDOUT | OWN_STDERR => MAX_DATA + 8,
        _ => MAX_CONTROL,
    }
}

pub(super) fn write_frame(writer: &mut impl Write, tag: u8, payload: &[u8]) -> io::Result<()> {
    if payload.len() > limit(tag) {
        return Err(invalid("attach frame exceeds its byte limit"));
    }
    let size = u32::try_from(payload.len()).map_err(|_| invalid("attach frame is too large"))?;
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(tag);
    frame.extend_from_slice(&size.to_be_bytes());
    frame.extend_from_slice(payload);
    writer.write_all(&frame)?;
    writer.flush()
}

pub(super) fn write_json(
    writer: &mut impl Write,
    tag: u8,
    value: &impl Serialize,
) -> io::Result<()> {
    let payload = serde_json::to_vec(value).map_err(io::Error::other)?;
    write_frame(writer, tag, &payload)
}

pub(super) fn read_frame(reader: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let (tag, size) = read_header(reader)?;
    if size > limit(tag) {
        return Err(invalid("attach frame exceeds its byte limit"));
    }
    Ok((tag, read_payload(reader, size)?))
}

/// The first frame of a connection, read before the client is known: any
/// other tag, or a hello above its own limit, is refused before its payload
/// is allocated or read.
pub(super) fn read_hello(reader: &mut impl Read) -> io::Result<Vec<u8>> {
    let (tag, size) = read_header(reader)?;
    if tag != HELLO || size > MAX_HELLO {
        return Err(invalid("the first attach frame is not a hello"));
    }
    read_payload(reader, size)
}

fn read_header(reader: &mut impl Read) -> io::Result<(u8, usize)> {
    let mut header = [0u8; 5];
    reader.read_exact(&mut header)?;
    let size = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    Ok((header[0], size))
}

fn read_payload(reader: &mut impl Read, size: usize) -> io::Result<Vec<u8>> {
    let mut payload = vec![0u8; size];
    reader.read_exact(&mut payload)?;
    Ok(payload)
}

pub(super) fn decode<T: DeserializeOwned>(payload: &[u8]) -> io::Result<T> {
    serde_json::from_slice(payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Own command output: the request tag, then the bytes.
pub(super) fn tagged(tag: u64, bytes: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(8 + bytes.len());
    payload.extend_from_slice(&tag.to_be_bytes());
    payload.extend_from_slice(bytes);
    payload
}

pub(super) fn untag(payload: &[u8]) -> io::Result<(u64, &[u8])> {
    let (tag, bytes) = payload
        .split_first_chunk::<8>()
        .ok_or_else(|| invalid("output frame without its request tag"))?;
    Ok((u64::from_be_bytes(*tag), bytes))
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::{
        decode, read_frame, read_hello, tagged, untag, write_frame, write_json, Hello, Text, HELLO,
        INPUT, MAX_DATA, NOTICE, OWN_STDOUT, PROTOCOL_VERSION_COMMANDS, PROTOCOL_VERSION_TERMINAL,
        SUBMIT, TERM_INPUT, TERM_OUTPUT, TERM_RESIZE,
    };

    #[test]
    fn shell_session_task_frames_roundtrip_and_enforce_limits() {
        let mut buffer = Vec::new();
        write_json(&mut buffer, NOTICE, &Text { text: "hi".into() }).unwrap();
        write_frame(&mut buffer, OWN_STDOUT, &tagged(7, b"out")).unwrap();
        let mut reader = buffer.as_slice();
        let (tag, payload) = read_frame(&mut reader).unwrap();
        assert_eq!(tag, NOTICE);
        assert_eq!(decode::<Text>(&payload).unwrap().text, "hi");
        let (tag, payload) = read_frame(&mut reader).unwrap();
        assert_eq!(tag, OWN_STDOUT);
        assert_eq!(untag(&payload).unwrap(), (7, &b"out"[..]));
        assert!(untag(b"short").is_err());
        assert!(write_frame(&mut Vec::new(), INPUT, &vec![0u8; MAX_DATA + 1]).is_err());
        // A peer announcing an oversized frame is refused before allocation.
        let mut oversized = vec![INPUT];
        oversized.extend_from_slice(&u32::MAX.to_be_bytes());
        assert!(read_frame(&mut oversized.as_slice()).is_err());
        // The first frame of a connection must be a hello within the hello
        // limit; any other header is refused before a payload is read, so the
        // error is invalid data rather than a missing payload.
        let sent = Hello {
            version: PROTOCOL_VERSION_COMMANDS,
            session: "id".into(),
            token: "ab".into(),
            label: "me".into(),
        };
        let mut hello = Vec::new();
        write_json(&mut hello, HELLO, &sent).unwrap();
        let payload = read_hello(&mut hello.as_slice()).unwrap();
        assert_eq!(decode::<Hello>(&payload).unwrap().token, "ab");
        for header in [
            [SUBMIT, 0x00, 0x80, 0x00, 0x00],
            [HELLO, 0x00, 0x00, 0x10, 0x01],
            [NOTICE, 0x00, 0x00, 0x00, 0x00],
        ] {
            let refused = read_hello(&mut header.as_slice()).unwrap_err();
            assert_eq!(refused.kind(), std::io::ErrorKind::InvalidData);
        }
    }

    #[test]
    fn terminal_task_frames_carry_raw_bytes_within_the_data_limit() {
        assert_eq!(PROTOCOL_VERSION_COMMANDS, 1);
        assert_eq!(PROTOCOL_VERSION_TERMINAL, 2);
        let bytes: Vec<u8> = (0..=255).collect();
        let mut buffer = Vec::new();
        write_frame(&mut buffer, TERM_INPUT, &bytes).unwrap();
        write_frame(&mut buffer, TERM_OUTPUT, &bytes).unwrap();
        let mut reader = buffer.as_slice();
        assert_eq!(
            read_frame(&mut reader).unwrap(),
            (TERM_INPUT, bytes.clone())
        );
        assert_eq!(read_frame(&mut reader).unwrap(), (TERM_OUTPUT, bytes));
        for tag in [TERM_INPUT, TERM_OUTPUT] {
            assert!(write_frame(&mut Vec::new(), tag, &vec![0u8; MAX_DATA]).is_ok());
            assert!(write_frame(&mut Vec::new(), tag, &vec![0u8; MAX_DATA + 1]).is_err());
            let mut oversized = vec![tag];
            oversized.extend_from_slice(&u32::MAX.to_be_bytes());
            assert!(read_frame(&mut oversized.as_slice()).is_err());
        }
        // A size report is a small control frame.
        assert!(write_frame(&mut Vec::new(), TERM_RESIZE, b"{\"cols\":80,\"rows\":24}").is_ok());
    }
}
