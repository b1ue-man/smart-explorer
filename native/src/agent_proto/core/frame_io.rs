//! Length-prefixed transport I/O for the unchanged agent wire format.
use std::io::{self, Read, Write};

use super::super::node_codec::{TreeDecodeBudget, TREE_BUDGET_ERROR};
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
    read_frame_with_tree_budget(r, |_| None)
}

pub fn read_frame_with_tree_budget(
    r: &mut impl Read,
    mut budget_for: impl FnMut(u64) -> Option<TreeDecodeBudget>,
) -> io::Result<Option<(u64, Frame)>> {
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
    if len < 9 {
        return Err(bad("truncated frame"));
    }
    let mut head = [0; 9];
    r.read_exact(&mut head)?;
    let id = u64::from_le_bytes(
        head[..8]
            .try_into()
            .map_err(|_| bad("invalid request id"))?,
    );
    let budget = if head[8] == 8 { budget_for(id) } else { None };
    if budget.is_some_and(|limit| len > limit.frame_bytes()) {
        let remaining = len - head.len();
        let copied = io::copy(&mut r.take(remaining as u64), &mut io::sink())?;
        if copied != remaining as u64 {
            return Err(bad("eof inside length"));
        }
        return Ok(Some((id, Frame::Err(TREE_BUDGET_ERROR.into()))));
    }
    let mut body = vec![0u8; len];
    body[..head.len()].copy_from_slice(&head);
    r.read_exact(&mut body[head.len()..])?;
    match Frame::decode_with_tree_budget(&body, budget) {
        Err(error) if error.to_string() == TREE_BUDGET_ERROR => {
            Ok(Some((id, Frame::Err(TREE_BUDGET_ERROR.into()))))
        }
        result => result.map(Some),
    }
}
