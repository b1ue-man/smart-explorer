//! The digest and size belong to the bytes read by this transfer.
use super::apply_guard::drift;
use super::snapshot_hash::md5_to_u64;
use super::types::Throttle;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Streamed {
    pub(crate) bytes: u64,
    pub(crate) digest: [u8; 16],
}
impl Streamed {
    pub(crate) fn hash(self) -> u64 {
        md5_to_u64(&self.digest)
    }
    pub(crate) fn hex(self) -> String {
        self.digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

pub(super) fn stream(
    reader: &mut dyn Read,
    writer: &mut dyn Write,
    cancel: &AtomicBool,
    throttle: Option<&Throttle>,
    expected_hash: u64,
    mut progress: impl FnMut(u64),
) -> io::Result<Streamed> {
    let mut context = md5::Context::new();
    let mut buffer = vec![0u8; 256 * 1024];
    let mut bytes = 0u64;
    loop {
        check(cancel)?;
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read])?;
        context.consume(&buffer[..read]);
        bytes = bytes
            .checked_add(read as u64)
            .ok_or_else(|| drift("transfer size overflow"))?;
        progress(read as u64);
        if let Some(throttle) = throttle {
            throttle.consume(read as u64);
        }
    }
    let result = Streamed {
        bytes,
        digest: context.compute().0,
    };
    if expected_hash != 0 && expected_hash != result.hash() {
        return Err(drift("file content changed since planning"));
    }
    Ok(result)
}

pub(super) fn check(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "synchronization canceled",
        ))
    } else {
        Ok(())
    }
}

/// Native provider MD5 in the same legacy signature representation.
pub(crate) fn native_hash(hex: &str) -> u64 {
    super::snapshot_hash::md5_hex_to_u64(hex)
}
