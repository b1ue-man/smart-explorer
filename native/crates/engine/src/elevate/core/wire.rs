//! Frames between se and its elevated helper (`se elevated exec`): on Linux
//! over the helper's standard input and output (pkexec and sudo pass them
//! on), on Windows over two named pipes. A frame is one tag byte, the
//! payload length as `u32` little endian, and the payload (at most 64 KiB).
//!
//! The helper's first frame is [`FromHelper::Ready`] with [`READY_MAGIC`];
//! a launcher that never started the helper (dialog dismissed, no polkit
//! agent, sudo needs a password) writes no frame, so its exit status and
//! error output decide what the caller reports (`outcome.rs`).
use std::io::{self, Read, Write};

use super::types::ElevatedExit;

/// Payload of the first frame of a helper that runs this protocol.
pub const READY_MAGIC: &[u8] = b"se-elevate-v1";
/// Longest payload of one frame.
pub const MAX_PAYLOAD: usize = 64 * 1024;

const TAG_STDIN: u8 = 1;
const TAG_EOF: u8 = 2;
const TAG_CANCEL: u8 = 3;
const TAG_READY: u8 = 0x10;
const TAG_STDOUT: u8 = 0x11;
const TAG_STDERR: u8 = 0x12;
const TAG_EXIT: u8 = 0x13;
const TAG_FAILED: u8 = 0x14;

/// From se to the helper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToHelper {
    Stdin(Vec<u8>),
    Eof,
    Cancel,
}

/// From the helper to se.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromHelper {
    Ready,
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Exit(ElevatedExit),
    /// The helper could not start the command; the text says why.
    Failed(String),
}

fn write_frame(writer: &mut dyn Write, tag: u8, payload: &[u8]) -> io::Result<()> {
    for chunk in chunks(payload) {
        let mut frame = Vec::with_capacity(5 + chunk.len());
        frame.push(tag);
        frame.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
        frame.extend_from_slice(chunk);
        writer.write_all(&frame)?;
    }
    writer.flush()
}

/// The payload in frames of at most [`MAX_PAYLOAD`]; an empty payload is
/// one empty frame.
fn chunks(payload: &[u8]) -> Vec<&[u8]> {
    if payload.is_empty() {
        return vec![payload];
    }
    payload.chunks(MAX_PAYLOAD).collect()
}

/// Reads one frame; `Ok(None)` at the end of the stream before a frame.
fn read_frame(reader: &mut dyn Read) -> io::Result<Option<(u8, Vec<u8>)>> {
    let mut tag = [0u8; 1];
    loop {
        match reader.read(&mut tag) {
            Ok(0) => return Ok(None),
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    let mut length = [0u8; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_PAYLOAD {
        return Err(invalid("elevated helper frame exceeds 64 KiB"));
    }
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload)?;
    Ok(Some((tag[0], payload)))
}

pub fn send_to_helper(writer: &mut dyn Write, frame: &ToHelper) -> io::Result<()> {
    match frame {
        ToHelper::Stdin(bytes) => write_frame(writer, TAG_STDIN, bytes),
        ToHelper::Eof => write_frame(writer, TAG_EOF, &[]),
        ToHelper::Cancel => write_frame(writer, TAG_CANCEL, &[]),
    }
}

pub fn read_to_helper(reader: &mut dyn Read) -> io::Result<Option<ToHelper>> {
    Ok(match read_frame(reader)? {
        None => None,
        Some((TAG_STDIN, payload)) => Some(ToHelper::Stdin(payload)),
        Some((TAG_EOF, _)) => Some(ToHelper::Eof),
        Some((TAG_CANCEL, _)) => Some(ToHelper::Cancel),
        Some((tag, _)) => return Err(invalid_tag(tag)),
    })
}

pub fn send_from_helper(writer: &mut dyn Write, frame: &FromHelper) -> io::Result<()> {
    match frame {
        FromHelper::Ready => write_frame(writer, TAG_READY, READY_MAGIC),
        FromHelper::Stdout(bytes) => write_frame(writer, TAG_STDOUT, bytes),
        FromHelper::Stderr(bytes) => write_frame(writer, TAG_STDERR, bytes),
        FromHelper::Exit(exit) => write_frame(writer, TAG_EXIT, &encode_exit(exit)),
        FromHelper::Failed(text) => {
            let text = text.as_bytes();
            write_frame(writer, TAG_FAILED, &text[..text.len().min(MAX_PAYLOAD)])
        }
    }
}

pub fn read_from_helper(reader: &mut dyn Read) -> io::Result<Option<FromHelper>> {
    Ok(match read_frame(reader)? {
        None => None,
        Some((TAG_READY, payload)) if payload == READY_MAGIC => Some(FromHelper::Ready),
        Some((TAG_READY, _)) => return Err(invalid("elevated helper speaks another protocol")),
        Some((TAG_STDOUT, payload)) => Some(FromHelper::Stdout(payload)),
        Some((TAG_STDERR, payload)) => Some(FromHelper::Stderr(payload)),
        Some((TAG_EXIT, payload)) => Some(FromHelper::Exit(decode_exit(&payload)?)),
        Some((TAG_FAILED, payload)) => Some(FromHelper::Failed(
            String::from_utf8_lossy(&payload).into_owned(),
        )),
        Some((tag, _)) => return Err(invalid_tag(tag)),
    })
}

/// `[flags][code i32][signal i32]`, flags bit 0 code, bit 1 signal, bit 2
/// cancelled, then the error text (empty: none).
fn encode_exit(exit: &ElevatedExit) -> Vec<u8> {
    let mut flags = 0u8;
    if exit.code.is_some() {
        flags |= 1;
    }
    if exit.signal.is_some() {
        flags |= 2;
    }
    if exit.cancelled {
        flags |= 4;
    }
    let mut bytes = vec![flags];
    bytes.extend_from_slice(&exit.code.unwrap_or(0).to_le_bytes());
    bytes.extend_from_slice(&exit.signal.unwrap_or(0).to_le_bytes());
    if let Some(error) = &exit.error {
        let error = error.as_bytes();
        bytes.extend_from_slice(&error[..error.len().min(MAX_PAYLOAD - 9)]);
    }
    bytes
}

fn decode_exit(bytes: &[u8]) -> io::Result<ElevatedExit> {
    if bytes.len() < 9 {
        return Err(invalid("elevated helper exit frame is too short"));
    }
    let flags = bytes[0];
    let number =
        |at: usize| i32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let error = (bytes.len() > 9).then(|| String::from_utf8_lossy(&bytes[9..]).into_owned());
    Ok(ElevatedExit {
        code: (flags & 1 != 0).then(|| number(1)),
        signal: (flags & 2 != 0).then(|| number(5)),
        cancelled: flags & 4 != 0,
        error,
    })
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn invalid_tag(tag: u8) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("unknown elevated helper frame {tag:#x}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_elevate_task_frames_round_trip_both_directions() {
        let mut buffer = Vec::new();
        for frame in [
            ToHelper::Stdin(b"input".to_vec()),
            ToHelper::Eof,
            ToHelper::Cancel,
        ] {
            send_to_helper(&mut buffer, &frame).unwrap();
        }
        let mut reader = buffer.as_slice();
        assert_eq!(
            read_to_helper(&mut reader).unwrap(),
            Some(ToHelper::Stdin(b"input".to_vec()))
        );
        assert_eq!(read_to_helper(&mut reader).unwrap(), Some(ToHelper::Eof));
        assert_eq!(read_to_helper(&mut reader).unwrap(), Some(ToHelper::Cancel));
        assert_eq!(read_to_helper(&mut reader).unwrap(), None);

        let exit = ElevatedExit {
            code: Some(-3),
            signal: None,
            cancelled: true,
            error: Some("gone".into()),
        };
        let mut buffer = Vec::new();
        for frame in [
            FromHelper::Ready,
            FromHelper::Stdout(vec![0, 1, 2]),
            FromHelper::Stderr(Vec::new()),
            FromHelper::Exit(exit.clone()),
            FromHelper::Failed("no".into()),
        ] {
            send_from_helper(&mut buffer, &frame).unwrap();
        }
        let mut reader = buffer.as_slice();
        assert_eq!(
            read_from_helper(&mut reader).unwrap(),
            Some(FromHelper::Ready)
        );
        assert_eq!(
            read_from_helper(&mut reader).unwrap(),
            Some(FromHelper::Stdout(vec![0, 1, 2]))
        );
        assert_eq!(
            read_from_helper(&mut reader).unwrap(),
            Some(FromHelper::Stderr(Vec::new()))
        );
        assert_eq!(
            read_from_helper(&mut reader).unwrap(),
            Some(FromHelper::Exit(exit))
        );
        assert_eq!(
            read_from_helper(&mut reader).unwrap(),
            Some(FromHelper::Failed("no".into()))
        );
        assert_eq!(read_from_helper(&mut reader).unwrap(), None);
    }

    #[test]
    fn shell_elevate_task_large_output_is_split_and_garbage_is_refused() {
        let mut buffer = Vec::new();
        let big = vec![7u8; MAX_PAYLOAD + 10];
        send_from_helper(&mut buffer, &FromHelper::Stdout(big)).unwrap();
        let mut reader = buffer.as_slice();
        let mut total = 0;
        while let Some(FromHelper::Stdout(bytes)) = read_from_helper(&mut reader).unwrap() {
            assert!(bytes.len() <= MAX_PAYLOAD);
            total += bytes.len();
        }
        assert_eq!(total, MAX_PAYLOAD + 10);
        // Text a launcher printed instead of the helper's first frame.
        let mut text: &[u8] = b"Error executing command as another user";
        assert!(read_from_helper(&mut text).is_err());
        let mut oversized = vec![TAG_STDOUT];
        oversized.extend_from_slice(&(MAX_PAYLOAD as u32 + 1).to_le_bytes());
        assert!(read_from_helper(&mut oversized.as_slice()).is_err());
        let mut wrong_magic = Vec::new();
        write_frame(&mut wrong_magic, TAG_READY, b"other").unwrap();
        assert!(read_from_helper(&mut wrong_magic.as_slice()).is_err());
    }
}
