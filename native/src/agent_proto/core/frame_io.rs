//! Length-prefixed transport I/O for the unchanged agent wire format.
use std::io::{self, Read, Write};

use super::super::types::Frame;
use super::{bad, frame_encode, validate_frame_len};

pub fn write_frame(w: &mut impl Write, req_id: u64, frame: &Frame) -> io::Result<()> {
    let bytes = frame_encode::encode(frame, req_id, true)?;
    // write_all retains short-write/Interrupted handling. An accepting writer
    // receives the length and payload together instead of a four-byte send.
    w.write_all(&bytes)?;
    w.flush()
}

pub fn read_frame(r: &mut impl Read) -> io::Result<Option<(u64, Frame)>> {
    let mut lenb = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match r.read(&mut lenb[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(bad("eof inside length")),
            Ok(n) => got += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    let len = u32::from_le_bytes(lenb) as usize;
    validate_frame_len(len)?;
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok(Some(Frame::decode(&body)?))
}
