//! Where the FFmpeg DLLs of the Windows viewer come from (plan 2026-11-17
//! P8). The installer puts the pinned DLLs into `<install dir>\ffmpeg\`
//! (`native/installer.nsi`); an installation that was updated by the
//! auto-updater (whose payloads are the program files only) or a portable
//! copy gets the same pinned files on its own: at the viewer's start a
//! background thread downloads the pinned archive once, checks its size and
//! SHA-256, takes the three DLLs and the licence out, checks each against
//! its pin and stores them in `<app data>\screen\ffmpeg\<release>\`
//! (`ffmpeg_fetch.rs`), the way Dokany reaches auto-updated installations
//! (`docs/RELEASING.md`). Until then, and wherever the download fails, Media
//! Foundation decodes. Every directory is checked against the pins (size and
//! SHA-256 of each DLL) before anything of it loads.
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

use crate::screen::core::ffmpeg_plan::{DLLS, FFMPEG_RELEASE};

const PINNED_MANIFEST: &str = include_str!("../../../../../../ffmpeg-runtime.nsh");

/// One pinned file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PinnedFile {
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// The pinned archive and the files taken from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Pins {
    pub url: &'static str,
    pub zip_size: u64,
    pub zip_sha256: &'static str,
    pub root: &'static str,
    /// In the order of `ffmpeg_plan::DLLS`.
    pub dlls: [PinnedFile; 3],
    pub license: PinnedFile,
}

fn manifest_value(name: &str) -> io::Result<&'static str> {
    let mut found = PINNED_MANIFEST.lines().filter_map(|line| {
        let rest = line.trim().strip_prefix("!define")?.trim_start();
        let rest = rest.strip_prefix(name)?;
        if rest.chars().next().is_some_and(|c| !c.is_whitespace()) {
            return None;
        }
        let quoted = rest.trim_start().strip_prefix('"')?;
        let (value, tail) = quoted.split_once('"')?;
        tail.trim().is_empty().then_some(value)
    });
    let invalid = |text: String| io::Error::new(io::ErrorKind::InvalidData, text);
    let value = found
        .next()
        .ok_or_else(|| invalid(format!("FFmpeg manifest lacks {name}")))?;
    if found.next().is_some() {
        return Err(invalid(format!("FFmpeg manifest defines {name} twice")));
    }
    Ok(value)
}

fn hex_ok(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn pinned_file(prefix: &str, name: Option<&'static str>) -> io::Result<PinnedFile> {
    let name = match name {
        Some(name) => name,
        None => manifest_value(&format!("{prefix}_DLL"))?,
    };
    let size = manifest_value(&format!("{prefix}_SIZE"))?
        .parse::<u64>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "FFmpeg pin size"))?;
    let sha256 = manifest_value(&format!("{prefix}_SHA256"))?;
    if size == 0 || !hex_ok(sha256) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("FFmpeg pin of {name} is invalid"),
        ));
    }
    Ok(PinnedFile { name, size, sha256 })
}

fn read_pins() -> io::Result<Pins> {
    let release = manifest_value("FFMPEG_RELEASE")?;
    let file = manifest_value("FFMPEG_ZIP_FILENAME")?;
    let url = manifest_value("FFMPEG_ZIP_URL")?;
    let root = manifest_value("FFMPEG_ZIP_ROOT")?;
    let zip = pinned_file("FFMPEG_ZIP", Some(file))?;
    let dlls = [
        pinned_file("FFMPEG_AVUTIL", None)?,
        pinned_file("FFMPEG_SWRESAMPLE", None)?,
        pinned_file("FFMPEG_AVCODEC", None)?,
    ];
    let license = pinned_file("FFMPEG_LICENSE", Some("LICENSE.txt"))?;
    let prefix = "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-";
    if release != FFMPEG_RELEASE
        || !url.starts_with(prefix)
        || !url.ends_with(&format!("/{file}"))
        || root != file.trim_end_matches(".zip")
        || dlls.iter().map(|dll| dll.name).ne(DLLS)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "FFmpeg manifest does not match the loader",
        ));
    }
    Ok(Pins {
        url,
        zip_size: zip.size,
        zip_sha256: zip.sha256,
        root,
        dlls,
        license,
    })
}

/// The pins of `native/ffmpeg-runtime.nsh`, read once.
pub(super) fn pins() -> io::Result<&'static Pins> {
    static PINS: OnceLock<Result<Pins, String>> = OnceLock::new();
    PINS.get_or_init(|| read_pins().map_err(|error| error.to_string()))
        .as_ref()
        .map_err(|reason| io::Error::new(io::ErrorKind::InvalidData, reason.clone()))
}

/// SHA-256 of `path` as lower-case hex, reading at most `limit` bytes.
pub(super) fn sha256_file(path: &Path, limit: u64) -> io::Result<(u64, String)> {
    let file = std::fs::File::open(path)?;
    let mut reader = file.take(limit.saturating_add(1));
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok((total, hex(&hasher.finalize())))
}

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Whether `dir` holds the three pinned DLLs, byte for byte.
pub(super) fn verify_dir(dir: &Path, pins: &Pins) -> io::Result<()> {
    for dll in &pins.dlls {
        let path = dir.join(dll.name);
        let (size, sha256) = sha256_file(&path, dll.size)?;
        if size != dll.size || sha256 != dll.sha256 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} does not match its pin", path.display()),
            ));
        }
    }
    Ok(())
}

/// `<install dir>\ffmpeg\`, next to the running program.
fn installed_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join("ffmpeg"))
}

/// `<app data>\screen\ffmpeg\<release>\`, the copy of this user.
pub(super) fn user_dir() -> PathBuf {
    crate::support_dirs::app_data_dir()
        .join("screen")
        .join("ffmpeg")
        .join(FFMPEG_RELEASE)
}

/// The directories the loader tries, in order.
pub(super) fn candidate_dirs() -> Vec<PathBuf> {
    installed_dir()
        .into_iter()
        .chain(std::iter::once(user_dir()))
        .collect()
}

/// `SE_FFMPEG_DIR`: a user's own, interface-compatible FFmpeg build (LGPL
/// v3 section 4 d 1, `third-party/ffmpeg/NOTICE.txt`), loaded without the
/// pins; the loader still checks major versions and layouts.
pub(super) const USER_BUILD_VARIABLE: &str = "SE_FFMPEG_DIR";

/// The first candidate directory whose DLLs match their pins (or the
/// user's own build), with the reasons of the others.
pub(super) fn verified_dir() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os(USER_BUILD_VARIABLE).map(PathBuf::from) {
        if DLLS.iter().all(|name| dir.join(name).is_file()) {
            return Ok(dir);
        }
    }
    let pins = pins().map_err(|error| error.to_string())?;
    let mut reasons = Vec::new();
    for dir in candidate_dirs() {
        match verify_dir(&dir, pins) {
            Ok(()) => return Ok(dir),
            Err(error) => reasons.push(format!("{}: {error}", dir.display())),
        }
    }
    Err(reasons.join("; "))
}

/// Loads FFmpeg in the background once per process and, where no
/// directory verifies, downloads this user's copy; a finished download makes
/// the loader try again (`ffmpeg_ffi::installed`).
pub(super) fn ensure_in_background() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("ffmpeg-runtime".into())
        .spawn(|| {
            // Loads (and checks) an existing copy before the first hello asks.
            if super::ffmpeg_ffi::present() || verified_dir().is_ok() {
                return;
            }
            match pins().and_then(|pins| super::ffmpeg_fetch::install(pins, &user_dir())) {
                Ok(()) => {
                    super::ffmpeg_ffi::installed();
                    let _ = super::ffmpeg_ffi::present();
                }
                Err(error) => eprintln!("FFmpeg-Laufzeit: {error}"),
            }
        });
}
