//! PROPFIND bodies exceed ureq's fixed string cap only within the shared
//! RAM budget. Reservations live through UTF-8 decoding and XML parsing.
use crate::transfer::MemoryReservation;
use std::io::{self, Read};
use std::ops::Deref;

pub(super) struct Body {
    text: String,
    _memory: Vec<MemoryReservation>,
}

impl Deref for Body {
    type Target = str;
    fn deref(&self) -> &str { &self.text }
}

impl std::fmt::Debug for Body {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebdavBody").field("bytes", &self.text.len()).finish()
    }
}

pub(super) fn read(response: ureq::Response) -> io::Result<Body> {
    let limit = usize::try_from(crate::transfer::memory_budget() / 8)
        .map_err(|_| memory_error("WebDAV XML budget exceeds the address space"))?;
    let mut reader = response.into_reader().take(limit as u64 + 1);
    let mut bytes = Vec::new();
    let mut reservations = Vec::new();
    let mut reserved = 0usize;
    let mut chunk = [0u8; 8 * 1024];
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 { break }
        let needed = bytes.len().checked_add(count).ok_or_else(|| memory_error("WebDAV XML length overflow"))?;
        if needed > limit {
            return Err(memory_error(&format!("WebDAV PROPFIND exceeds its RAM-derived {limit}-byte XML limit")));
        }
        if needed > reserved {
            let next = needed.max(reserved.saturating_mul(2)).min(limit);
            // Account for the body and leave a conservative parsing allowance.
            // Grow by actual need, so small listings do not reserve the cap.
            let memory = crate::transfer::try_reserve_memory((next - reserved) as u64 * 4)
                .ok_or_else(|| memory_error("WebDAV XML shared memory budget is occupied"))?;
            bytes.try_reserve_exact(next - bytes.len())
                .map_err(|_| memory_error("WebDAV XML allocation failed"))?;
            reservations.push(memory);
            reserved = next;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    let text = String::from_utf8(bytes).map_err(|_| io::Error::new(io::ErrorKind::InvalidData,
        "WebDAV XML is not UTF-8"))?;
    Ok(Body { text, _memory: reservations })
}

fn memory_error(detail: &str) -> io::Error { io::Error::new(io::ErrorKind::OutOfMemory, detail) }
