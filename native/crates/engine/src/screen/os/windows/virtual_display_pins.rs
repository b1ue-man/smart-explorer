//! The pinned virtual display driver (plan 2026-11-17 P5,
//! `native/vdd-driver.nsh`): the manifest is read here, the archive is
//! downloaded into memory (132 KB) over HTTPS with at most two redirects and
//! only to the pinned address or GitHub's release asset hosts (as the FFmpeg
//! and Dokany downloads), no byte beyond the pinned size is taken, then size
//! and SHA-256 of the archive and of each of its three driver files must
//! match. Nothing the user's account could have written is trusted: the
//! elevated step downloads by itself.
use std::io::{self, Read};
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

use crate::screen::core::virtual_display::windows_driver::driver_hash;

const MANIFEST: &str = include_str!("../../../../../../vdd-driver.nsh");
const TRUSTED_HOSTS: [&str; 2] = [
    "https://release-assets.githubusercontent.com/",
    "https://objects.githubusercontent.com/",
];
const URL_PREFIX: &str =
    "https://github.com/VirtualDrivers/Virtual-Display-Driver/releases/download/";
const EOCD: u32 = 0x0605_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;

/// One pinned file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinnedFile {
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// The pinned archive and its driver files (catalog, DLL, INF).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pins {
    pub release: &'static str,
    pub url: &'static str,
    pub zip_size: u64,
    pub zip_sha256: &'static str,
    pub root: &'static str,
    pub files: [PinnedFile; 3],
}

impl Pins {
    /// `driver_hash` of the pinned files: the identity the installed state
    /// is compared with (no download needed).
    pub fn driver_hash(&self) -> String {
        let pairs: Vec<(&str, &str)> = self.files.iter().map(|f| (f.name, f.sha256)).collect();
        driver_hash(&pairs)
    }

    pub fn inf(&self) -> &PinnedFile {
        &self.files[2]
    }
}

fn invalid(text: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, text.into())
}

fn value(name: &str) -> io::Result<&'static str> {
    let mut found = MANIFEST.lines().filter_map(|line| {
        let rest = line.trim().strip_prefix("!define")?.trim_start();
        let rest = rest.strip_prefix(name)?;
        if rest.chars().next().is_some_and(|c| !c.is_whitespace()) {
            return None;
        }
        let (value, tail) = rest.trim_start().strip_prefix('"')?.split_once('"')?;
        tail.trim().is_empty().then_some(value)
    });
    let value = found
        .next()
        .ok_or_else(|| invalid(format!("driver manifest lacks {name}")))?;
    if found.next().is_some() {
        return Err(invalid(format!("driver manifest defines {name} twice")));
    }
    Ok(value)
}

fn hex_ok(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn pinned(prefix: &str, name: &'static str) -> io::Result<PinnedFile> {
    let size = value(&format!("{prefix}_SIZE"))?
        .parse::<u64>()
        .map_err(|_| invalid(format!("driver pin size of {name}")))?;
    let sha256 = value(&format!("{prefix}_SHA256"))?;
    if size == 0 || size > 16 << 20 || !hex_ok(sha256) {
        return Err(invalid(format!("driver pin of {name} is invalid")));
    }
    Ok(PinnedFile { name, size, sha256 })
}

fn read_pins() -> io::Result<Pins> {
    let release = value("VDD_RELEASE")?;
    let file = value("VDD_ZIP_FILENAME")?;
    let url = value("VDD_ZIP_URL")?;
    let zip = pinned("VDD_ZIP", file)?;
    if url != format!("{URL_PREFIX}{release}/{file}") {
        return Err(invalid("driver manifest URL does not match its release"));
    }
    Ok(Pins {
        release,
        url,
        zip_size: zip.size,
        zip_sha256: zip.sha256,
        root: value("VDD_ZIP_ROOT")?,
        files: [
            pinned("VDD_CAT", value("VDD_CAT_FILE")?)?,
            pinned("VDD_DLL", value("VDD_DLL_FILE")?)?,
            pinned("VDD_INF", value("VDD_INF_FILE")?)?,
        ],
    })
}

/// The pins of `native/vdd-driver.nsh`, read once.
pub fn pins() -> io::Result<&'static Pins> {
    static PINS: OnceLock<Result<Pins, String>> = OnceLock::new();
    PINS.get_or_init(|| read_pins().map_err(|error| error.to_string()))
        .as_ref()
        .map_err(|reason| invalid(reason.clone()))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn download(pins: &Pins) -> io::Result<Vec<u8>> {
    let final_url = |actual: &str| {
        actual == pins.url || TRUSTED_HOSTS.iter().any(|host| actual.starts_with(host))
    };
    let source = crate::http_resume::Source {
        url: pins.url,
        user_agent: "SmartExplorer-Display-Driver/1",
        https_only: true,
        redirects: 2,
        final_url: &final_url,
    };
    let bound = crate::http_resume::Bound {
        exact: Some(pins.zip_size),
        ceiling: None,
    };
    let mut bytes = Vec::with_capacity(pins.zip_size as usize);
    crate::http_resume::fetch(&source, bound, &mut bytes, &mut |_, _| {}, &|| false)
        .map_err(|error| io::Error::new(error.kind(), format!("driver download: {error}")))?;
    if bytes.len() as u64 != pins.zip_size || sha256_hex(&bytes) != pins.zip_sha256 {
        return Err(invalid("driver archive does not match its pin"));
    }
    Ok(bytes)
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

/// The entry `name` of the archive in memory, checked against `pin`.
fn extract(zip: &[u8], name: &str, pin: &PinnedFile) -> io::Result<Vec<u8>> {
    let end = (0..zip.len().saturating_sub(21))
        .rev()
        .find(|at| u32_at(zip, *at).ok() == Some(EOCD))
        .ok_or_else(|| invalid("ZIP end record missing"))?;
    let count = u16_at(zip, end + 10)? as usize;
    let mut at = u32_at(zip, end + 16)? as usize;
    for _ in 0..count {
        if u32_at(zip, at)? != CENTRAL {
            return Err(invalid("ZIP central directory entry expected"));
        }
        let name_length = u16_at(zip, at + 28)? as usize;
        let skip = u16_at(zip, at + 30)? as usize + u16_at(zip, at + 32)? as usize;
        let entry = zip
            .get(at + 46..at + 46 + name_length)
            .ok_or_else(|| invalid("ZIP entry name cut short"))?;
        if entry == name.as_bytes() {
            let method = u16_at(zip, at + 10)?;
            let compressed = u32_at(zip, at + 20)? as usize;
            let local = u32_at(zip, at + 42)? as usize;
            if u32_at(zip, local)? != LOCAL || !matches!(method, 0 | 8) {
                return Err(invalid(format!("{name}: unsupported ZIP entry")));
            }
            let data =
                local + 30 + u16_at(zip, local + 26)? as usize + u16_at(zip, local + 28)? as usize;
            let raw = zip
                .get(data..data + compressed)
                .ok_or_else(|| invalid(format!("{name}: ZIP data cut short")))?;
            let mut out = Vec::with_capacity(pin.size as usize);
            if method == 8 {
                flate2::read::DeflateDecoder::new(raw)
                    .take(pin.size + 1)
                    .read_to_end(&mut out)?;
            } else {
                out.extend_from_slice(raw);
            }
            if out.len() as u64 != pin.size || sha256_hex(&out) != pin.sha256 {
                return Err(invalid(format!("{name} does not match its pin")));
            }
            return Ok(out);
        }
        at += 46 + name_length + skip;
    }
    Err(invalid(format!("{name} is not in the driver archive")))
}

/// Downloads the pinned archive and returns its three driver files (in the
/// order of `Pins::files`), each checked against its pin.
pub fn fetch_driver(pins: &Pins) -> io::Result<Vec<(&'static str, Vec<u8>)>> {
    let zip = download(pins)?;
    pins.files
        .iter()
        .map(|pin| {
            extract(&zip, &format!("{}/{}", pins.root, pin.name), pin)
                .map(|bytes| (pin.name, bytes))
        })
        .collect()
}
