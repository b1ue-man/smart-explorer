//! Whole-file copies between the spool and the backend with large blocks.
//! `std::io::copy` moves 8 KiB per call between generic streams, which on a
//! drive is one agent frame (and one request) per 8 KiB; 1 MiB blocks let
//! the backend stream fill its full frames (G4 ML6).
use std::io::{self, Read, Write};

/// Block size of spool uploads and downloads.
pub(super) const BLOCK: usize = 1024 * 1024;

/// Copies `reader` to its end into `writer`; returns the bytes copied.
pub(super) fn copy(reader: &mut dyn Read, writer: &mut dyn Write) -> io::Result<u64> {
    let mut buffer = vec![0u8; BLOCK];
    let mut copied = 0u64;
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) => return Ok(copied),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        writer.write_all(&buffer[..read])?;
        copied = copied.saturating_add(read as u64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_task_drive_bulk_copy_keeps_every_byte_across_block_edges() {
        let data: Vec<u8> = (0..(BLOCK * 2 + 17)).map(|n| (n % 251) as u8).collect();
        let mut output = Vec::new();
        let copied = copy(&mut data.as_slice(), &mut output).unwrap();
        assert_eq!(copied, data.len() as u64);
        assert_eq!(output, data);
    }
}
