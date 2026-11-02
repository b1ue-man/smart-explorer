//! Deflating embedded files in build scripts (raw deflate, best compression),
//! shared by the build scripts of the core and transfer crates (D13).
use std::path::Path;

/// Writes `bytes` deflated (raw deflate, best compression) to `target`, after
/// proving that the program's inflation gives them back unchanged.
pub fn write_deflated(bytes: &[u8], target: &Path) {
    use std::io::{Read, Write};
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    encoder
        .write_all(bytes)
        .unwrap_or_else(|error| io_failed(target, error));
    let packed = encoder
        .finish()
        .unwrap_or_else(|error| io_failed(target, error));
    let mut back = Vec::with_capacity(bytes.len());
    flate2::read::DeflateDecoder::new(packed.as_slice())
        .read_to_end(&mut back)
        .unwrap_or_else(|error| io_failed(target, error));
    assert!(back == bytes, "deflate round trip of {}", target.display());
    std::fs::write(target, packed).unwrap_or_else(|error| io_failed(target, error));
}

fn io_failed(target: &Path, error: std::io::Error) -> ! {
    panic!("{}: {error}", target.display())
}
