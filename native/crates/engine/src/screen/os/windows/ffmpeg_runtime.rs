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
//!
//! Since 2026-11-24 the download also starts when the daemon starts
//! (`ensure_at_daemon`) and in every process whose loader finds no copy that
//! verifies (`verified_dir`: `se screen bench`, a session of the app, the
//! daemon's own sessions), not only in the viewer window. A failed attempt
//! is retried after 30 s, doubling up to one hour, and is recorded in
//! `<app data>\screen\ffmpeg\install.json`; `status` gives the state to
//! `se screen codec` and the codec page (`codec_runtime_install.rs`).
//! `install.lock` beside it lets one installation run at a time; it removes
//! what an installation of an ended process left behind.
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

use super::codec_runtime_install::{
    clear_record, held_elsewhere, next_record, read_record, run_background, unix_now, write_record,
    InstallLock, RuntimeReport, RuntimeState, Seen,
};
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

/// `<app data>\screen\ffmpeg\`: the copies, the record and the lock.
fn install_root() -> PathBuf {
    crate::support_dirs::app_data_dir()
        .join("screen")
        .join("ffmpeg")
}

/// `<app data>\screen\ffmpeg\<release>\`, the copy of this user.
pub(super) fn user_dir() -> PathBuf {
    install_root().join(FFMPEG_RELEASE)
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
    // No copy verifies: this process fetches this user's copy.
    fetch_on_demand();
    Err(reasons.join("; "))
}

/// Test builds start no download on their own (`codec_runtime_install.rs`).
/// A process that only loads keeps the schedule of a failed attempt.
#[cfg(not(any(test, feature = "test-support")))]
fn fetch_on_demand() {
    if super::codec_runtime_install::downloads_allowed() {
        start(false, false);
    }
}

#[cfg(any(test, feature = "test-support"))]
fn fetch_on_demand() {}

/// Loads FFmpeg in the background once per process and, where no
/// directory verifies, downloads this user's copy; a finished download makes
/// the loader try again (`ffmpeg_ffi::installed`). The viewer's start.
pub(super) fn ensure_in_background() {
    start(true, true);
}

/// The same at the start of the daemon, without loading the DLLs into it.
/// Test builds start no download on their own, so nothing calls it there.
#[cfg_attr(any(test, feature = "test-support"), allow(dead_code))]
pub(super) fn ensure_at_daemon() {
    start(false, true);
}

/// One background installer per process: `preload` loads an existing copy
/// before the first hello asks (the viewer); `at_once` attempts at once
/// even right after a failed attempt (a start).
fn start(preload: bool, at_once: bool) {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("ffmpeg-runtime".into())
        .spawn(move || install_in_background(preload, at_once));
}

fn install_in_background(preload: bool, at_once: bool) {
    let root = install_root();
    let look = |first: bool| Seen {
        opted_out: false,
        // Later rounds find a copy another process installed in `attempt`.
        installed: first
            && if preload {
                super::ffmpeg_ffi::present()
            } else {
                verified_dir().is_ok()
            },
        record: read_record(&root),
    };
    let attempt = || match install_now(&root) {
        Ok(()) => {
            super::ffmpeg_ffi::installed();
            if preload {
                let _ = super::ffmpeg_ffi::present();
            }
            Ok(())
        }
        Err(error) => {
            let reason = error.to_string();
            eprintln!("FFmpeg-Laufzeit: {reason}");
            let next = next_record(read_record(&root).as_ref(), &reason, unix_now());
            let _ = write_record(&root, &next);
            Err(reason)
        }
    };
    run_background(false, at_once, look, attempt);
}

/// One installation under the lock, after removing what an ended process
/// left behind.
fn install_now(root: &Path) -> io::Result<()> {
    let _lock = InstallLock::acquire(root)?;
    super::ffmpeg_fetch::remove_leftovers(root);
    pins().and_then(|pins| super::ffmpeg_fetch::install(pins, &user_dir()))?;
    let _ = clear_record(root);
    Ok(())
}

/// The state of this user's FFmpeg for `se screen codec` and the codec page.
pub(super) fn status() -> RuntimeReport {
    let root = install_root();
    let report = |state, reason, retry_at| RuntimeReport {
        state,
        version: FFMPEG_RELEASE.to_string(),
        reason,
        retry_at,
    };
    if super::ffmpeg_ffi::present() {
        return report(RuntimeState::Installed, None, None);
    }
    if held_elsewhere(&root) {
        return report(RuntimeState::Installing, None, None);
    }
    match read_record(&root) {
        Some(record) => report(
            RuntimeState::Failed,
            Some(record.reason),
            Some(record.retry_at),
        ),
        None => report(RuntimeState::Absent, None, None),
    }
}
