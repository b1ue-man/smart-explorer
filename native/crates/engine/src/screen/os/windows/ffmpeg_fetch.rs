//! The download of this user's FFmpeg copy (plan 2026-11-17 P8, see
//! `ffmpeg_runtime.rs`): HTTPS only, at most two redirects, and only the
//! pinned URL or GitHub's release asset hosts as the final address (as the
//! Dokany download, `mount/os/windows/runtime_install_download.rs`); no
//! byte beyond the pinned size is taken, then size and SHA-256 must match.
//! No limit on the whole transfer (user rule 2026-11-17): `http_resume`
//! limits the connect and each read and continues a broken transfer with a
//! range request. The three DLLs and the licence come out of the
//! archive with a reader of the ZIP central directory and raw deflate
//! (`flate2`, already a dependency), each checked against its pin, into a
//! fresh directory beside the target that is renamed into place at the end,
//! so a half-written copy is never loaded.
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use sha2::{Digest, Sha256};

use super::ffmpeg_runtime::{hex, sha256_file, verify_dir, PinnedFile, Pins};

const TRUSTED_HOSTS: [&str; 2] = [
    "https://release-assets.githubusercontent.com/",
    "https://objects.githubusercontent.com/",
];
const EOCD: u32 = 0x0605_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;

fn invalid(text: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, text.into())
}

fn trusted_final_url(actual: &str, original: &str) -> bool {
    actual == original || TRUSTED_HOSTS.iter().any(|host| actual.starts_with(host))
}

/// Downloads the archive into `file` and checks it against the pins.
fn download(pins: &Pins, file: &mut fs::File) -> io::Result<()> {
    let source = crate::http_resume::Source {
        url: pins.url,
        user_agent: "SmartExplorer-FFmpeg-Runtime/1",
        https_only: true,
        redirects: 2,
        final_url: &|actual| trusted_final_url(actual, pins.url),
    };
    let bound = crate::http_resume::Bound {
        exact: Some(pins.zip_size),
        ceiling: None,
    };
    let written = crate::http_resume::fetch(&source, bound, file, &mut |_, _| {}, &|| false)
        .map_err(|error| io::Error::new(error.kind(), format!("FFmpeg download: {error}")))?;
    if written != pins.zip_size {
        return Err(invalid(format!(
            "FFmpeg archive has {written} bytes instead of {}",
            pins.zip_size
        )));
    }
    file.sync_all()?;
    file.seek(SeekFrom::Start(0))?;
    let mut hasher = Sha256::new();
    io::copy(
        &mut (&*file).take(pins.zip_size),
        &mut HashWriter(&mut hasher),
    )?;
    if hex(&hasher.finalize()) != pins.zip_sha256 {
        return Err(invalid("FFmpeg archive does not match its SHA-256"));
    }
    Ok(())
}

struct HashWriter<'a>(&'a mut Sha256);

impl Write for HashWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn u16_at(bytes: &[u8], at: usize) -> io::Result<u16> {
    bytes
        .get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| invalid("ZIP record cut short"))
}

fn u32_at(bytes: &[u8], at: usize) -> io::Result<u32> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| invalid("ZIP record cut short"))
}

/// One entry of the central directory.
struct Entry {
    name: String,
    method: u16,
    compressed: u64,
    size: u64,
    local: u64,
}

/// The entries of the central directory (no ZIP64: the pinned archive has
/// 237 entries in 80 MB, the 32-bit fields hold every value).
fn central_directory(file: &mut fs::File, length: u64) -> io::Result<Vec<Entry>> {
    let tail = length.min(22 + 65_535);
    file.seek(SeekFrom::Start(length - tail))?;
    let mut end = vec![0u8; tail as usize];
    file.read_exact(&mut end)?;
    let at = (0..end.len().saturating_sub(21))
        .rev()
        .find(|at| u32_at(&end, *at).ok() == Some(EOCD))
        .ok_or_else(|| invalid("ZIP end record missing"))?;
    let count = u16_at(&end, at + 10)? as usize;
    let size = u64::from(u32_at(&end, at + 12)?);
    let offset = u64::from(u32_at(&end, at + 16)?);
    if offset.checked_add(size).is_none_or(|end| end > length) || size > 16 << 20 {
        return Err(invalid("ZIP central directory outside the archive"));
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut directory = vec![0u8; size as usize];
    file.read_exact(&mut directory)?;
    let mut entries = Vec::with_capacity(count);
    let mut at = 0usize;
    for _ in 0..count {
        if u32_at(&directory, at)? != CENTRAL {
            return Err(invalid("ZIP central directory entry expected"));
        }
        let name_length = u16_at(&directory, at + 28)? as usize;
        let skip = u16_at(&directory, at + 30)? as usize + u16_at(&directory, at + 32)? as usize;
        let name = directory
            .get(at + 46..at + 46 + name_length)
            .ok_or_else(|| invalid("ZIP entry name cut short"))?;
        entries.push(Entry {
            name: String::from_utf8_lossy(name).into_owned(),
            method: u16_at(&directory, at + 10)?,
            compressed: u64::from(u32_at(&directory, at + 20)?),
            size: u64::from(u32_at(&directory, at + 24)?),
            local: u64::from(u32_at(&directory, at + 42)?),
        });
        at += 46 + name_length + skip;
    }
    Ok(entries)
}

/// Writes the entry `name` of the archive to `target` and checks it
/// against `pin`.
fn extract(
    file: &mut fs::File,
    entries: &[Entry],
    name: &str,
    pin: &PinnedFile,
    target: &Path,
) -> io::Result<()> {
    let entry = entries
        .iter()
        .find(|entry| entry.name == name)
        .ok_or_else(|| invalid(format!("{name} is not in the FFmpeg archive")))?;
    if entry.size != pin.size || !matches!(entry.method, 0 | 8) {
        return Err(invalid(format!(
            "{name} in the archive does not match its pin"
        )));
    }
    let mut local = [0u8; 30];
    file.seek(SeekFrom::Start(entry.local))?;
    file.read_exact(&mut local)?;
    if u32_at(&local, 0)? != LOCAL {
        return Err(invalid("ZIP local header expected"));
    }
    let data = entry.local + 30 + u64::from(u16_at(&local, 26)?) + u64::from(u16_at(&local, 28)?);
    file.seek(SeekFrom::Start(data))?;
    let raw = (&*file).take(entry.compressed);
    let mut out = fs::File::create(target)?;
    let mut hasher = Sha256::new();
    let written = {
        let mut sink = Tee(&mut out, &mut hasher);
        if entry.method == 8 {
            io::copy(
                &mut flate2::read::DeflateDecoder::new(raw).take(pin.size + 1),
                &mut sink,
            )?
        } else {
            io::copy(&mut raw.take(pin.size + 1), &mut sink)?
        }
    };
    out.sync_all()?;
    if written != pin.size || hex(&hasher.finalize()) != pin.sha256 {
        return Err(invalid(format!("{name} does not match its pin")));
    }
    Ok(())
}

struct Tee<'a>(&'a mut fs::File, &'a mut Sha256);

impl Write for Tee<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.0.write(bytes)?;
        self.1.update(&bytes[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// Downloads the pinned archive and installs its DLLs and licence into
/// `target` (replacing nothing that already verifies).
pub(super) fn install(pins: &Pins, target: &Path) -> io::Result<()> {
    if verify_dir(target, pins).is_ok() {
        return Ok(());
    }
    let parent = target
        .parent()
        .ok_or_else(|| invalid("FFmpeg target has no parent"))?;
    fs::create_dir_all(parent)?;
    let mut archive = tempfile::Builder::new()
        .prefix("ffmpeg-")
        .suffix(".zip")
        .tempfile_in(parent)?;
    download(pins, archive.as_file_mut())?;
    let staging = tempfile::Builder::new()
        .prefix("ffmpeg-staging-")
        .tempdir_in(parent)?;
    let file = archive.as_file_mut();
    let entries = central_directory(file, pins.zip_size)?;
    for pin in pins.dlls.iter().chain(std::iter::once(&pins.license)) {
        let inner = if pin.name == "LICENSE.txt" {
            format!("{}/LICENSE.txt", pins.root)
        } else {
            format!("{}/bin/{}", pins.root, pin.name)
        };
        extract(file, &entries, &inner, pin, &staging.path().join(pin.name))?;
    }
    verify_dir(staging.path(), pins)?;
    let (size, sha256) = sha256_file(&staging.path().join("LICENSE.txt"), pins.license.size)?;
    if size != pins.license.size || sha256 != pins.license.sha256 {
        return Err(invalid("FFmpeg licence does not match its pin"));
    }
    if target.exists() {
        fs::remove_dir_all(target)?;
    }
    let staged = staging.keep();
    if let Err(error) = fs::rename(&staged, target) {
        let _ = fs::remove_dir_all(&staged);
        return Err(error);
    }
    Ok(())
}
