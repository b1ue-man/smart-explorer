//! The OpenH264 library of this user (SCR-M3; automatic since 2026-11-24):
//! Cisco's 2.6.0 binary installs on its own when the daemon starts
//! (`install_in_background`) and when a session needs H.264 that no other
//! coder has (`session_ready`); `se screen codec` and the codec page show
//! Cisco's notice and license (Cisco's conditions). The user may switch it
//! off (`disable`, kept in `state.json` and respected by every automatic
//! installation), on again (`enable`) and remove it (`remove`: the file
//! goes, OpenH264 stays off until `enable`). It lives in the private app
//! data folder `screen/openh264/` beside `state.json`, the record of a
//! failed attempt (`install.json`) and the lock of an installation
//! (`install.lock`, `codec_runtime_install.rs`). A download is HTTPS only,
//! from the pinned address without redirects, and is accepted only with
//! the pinned size and SHA-256; the unpacked library must match its pin as
//! well. Every load checks the file against its pin right before
//! (`verified_library`), so a changed file is never loaded.
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::codec_runtime_install::{
    clear_record, held_elsewhere, next_record, read_record, run_background, session_gate, unix_now,
    write_record, FirstContact, InstallLock, InstallRecord, Seen, Trigger,
};
use super::openh264_pins::{
    OpenH264Pin, CISCO_BINARY_LICENSE_SHA256, DOWNLOAD_HOST, MAX_LIBRARY_BYTES, PIN,
};
use crate::screen::core::codec_registry::{OpenH264Report, OpenH264State, OPENH264_VERSION};

const STATE_FILE: &str = "state.json";
const MAX_STATE_BYTES: u64 = 4096;
/// Test builds never download on their own (`codec_runtime_install.rs`).
#[cfg(not(any(test, feature = "test-support")))]
const AUTO_INSTALL: bool = true;
#[cfg(any(test, feature = "test-support"))]
const AUTO_INSTALL: bool = false;

/// `state.json`: the user's switch; without it OpenH264 is on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct StoredState {
    enabled: bool,
    version: String,
    /// The license text shown with the switch (its SHA-256).
    license_sha256: String,
    /// Unix seconds of the last switch.
    changed_at: i64,
}

/// Fetches the compressed library for `pin`.
pub type Fetch<'a> = &'a dyn Fn(&OpenH264Pin) -> io::Result<Vec<u8>>;

/// The folder with the library and its state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenH264Store {
    dir: PathBuf,
    /// Installs on demand of a session (this user's store).
    auto: bool,
}

pub fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn pin() -> io::Result<OpenH264Pin> {
    PIN.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "OpenH264 2.6.0 is not offered for this platform",
        )
    })
}

fn mismatch(what: &str, actual: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{what} does not match its pin (SHA-256 {actual})"),
    )
}

fn switched_off() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "OpenH264 is switched off")
}

impl OpenH264Store {
    /// The store of this user: `<app data>/screen/openh264`, installing on
    /// demand of a session.
    pub fn for_user() -> Self {
        let data = crate::support_dirs::app_data_dir();
        Self {
            dir: data.join("screen").join("openh264"),
            auto: AUTO_INSTALL,
        }
    }

    /// A store at `dir` that never installs on its own (task tests).
    pub fn at(dir: PathBuf) -> Self {
        Self { dir, auto: false }
    }

    /// A store at `dir` that installs on demand like this user's.
    pub fn auto_at(dir: PathBuf) -> Self {
        Self { dir, auto: true }
    }

    /// Whether a session installs this store's library when it needs it.
    pub fn installs_on_demand(&self) -> bool {
        self.auto && PIN.is_some()
    }

    /// Where the library lies; `None` where no pin fits the platform.
    pub fn library_path(&self) -> Option<PathBuf> {
        PIN.map(|pin| self.dir.join(pin.library_name))
    }

    pub fn library_present(&self) -> bool {
        self.library_path().is_some_and(|path| path.is_file())
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join(STATE_FILE)
    }

    fn read_state(&self) -> Option<StoredState> {
        let path = self.state_path();
        if !path.is_file() {
            return None;
        }
        let text = crate::support_dirs::read_private_text(&path, MAX_STATE_BYTES).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn write_state(&self, enabled: bool, license_sha256: &str) -> io::Result<()> {
        let state = StoredState {
            enabled,
            version: OPENH264_VERSION.into(),
            license_sha256: license_sha256.into(),
            changed_at: unix_now(),
        };
        let json = serde_json::to_vec_pretty(&state).map_err(io::Error::other)?;
        crate::support_dirs::write_private_atomic(&self.state_path(), &json)
    }

    /// Switched off by the user.
    pub fn opted_out(&self) -> bool {
        self.read_state().is_some_and(|state| !state.enabled)
    }

    /// Present and not switched off; the pin is checked at load.
    pub fn switched_on(&self) -> bool {
        self.library_present() && !self.opted_out()
    }

    /// The record of the last failed installation.
    pub fn record(&self) -> Option<InstallRecord> {
        read_record(&self.dir)
    }

    /// Whether the library is present and matches its pin.
    pub fn verified(&self) -> bool {
        self.library_path()
            .is_some_and(|path| verify(&path).is_ok())
    }

    /// The state for `se screen codec` and the codec view, with the pin
    /// check of the file.
    pub fn report(&self) -> OpenH264Report {
        let Some(path) = self.library_path() else {
            let reason = "OpenH264 2.6.0 is not offered for this platform";
            return OpenH264Report::new(OpenH264State::Absent, Some(reason.into()));
        };
        let present = path.is_file();
        let plain = |state| OpenH264Report::new(state, None).with_library(present);
        if self.opted_out() {
            return plain(OpenH264State::Disabled);
        }
        if held_elsewhere(&self.dir) {
            return plain(OpenH264State::Installing);
        }
        let record = self.record();
        if present {
            return match verify(&path) {
                Ok(()) => plain(OpenH264State::Enabled),
                Err(error) => OpenH264Report::new(OpenH264State::Failed, Some(error.to_string()))
                    .with_library(true)
                    .with_retry_at(record.map(|record| record.retry_at)),
            };
        }
        match record {
            Some(record) => OpenH264Report::new(OpenH264State::Failed, Some(record.reason))
                .with_retry_at(Some(record.retry_at)),
            None => plain(OpenH264State::Absent),
        }
    }

    /// The state without reading the library (a session's register).
    pub fn quick_report(&self) -> OpenH264Report {
        if PIN.is_none() {
            return self.report();
        }
        let present = self.library_present();
        let state = if self.opted_out() {
            OpenH264State::Disabled
        } else if present {
            OpenH264State::Enabled
        } else {
            OpenH264State::Absent
        };
        OpenH264Report::new(state, None).with_library(present)
    }

    /// The library after its SHA-256 matched the pin; nothing is handed out
    /// while the user switched OpenH264 off.
    pub fn verified_library(&self) -> io::Result<PathBuf> {
        let pin = pin()?;
        if self.opted_out() {
            return Err(switched_off());
        }
        let path = self.dir.join(pin.library_name);
        verify(&path)?;
        Ok(path)
    }

    /// Switches OpenH264 on again (`accepted`: the SHA-256 of the license
    /// text this build shows) and installs it now when it is missing or
    /// does not match its pin. A failed download keeps it switched on; the
    /// background installer and the next session try again.
    pub fn enable(&self, accepted: &str) -> io::Result<()> {
        self.enable_with(accepted, &download)
    }

    pub fn enable_with(&self, accepted: &str, fetch: Fetch<'_>) -> io::Result<()> {
        check_license(accepted)?;
        pin()?;
        self.write_state(true, accepted)?;
        self.ensure_with(Trigger::User, &unix_now, fetch).map(drop)
    }

    /// The same with the compressed file fetched elsewhere (the task suite,
    /// an offline copy): it must match the pinned size and SHA-256 as well.
    pub fn enable_from_compressed(&self, accepted: &str, compressed: &[u8]) -> io::Result<()> {
        check_license(accepted)?;
        let pin = pin()?;
        self.install(&pin, compressed)?;
        self.write_state(true, accepted)
    }

    /// Switches OpenH264 off; a present file stays for switching on again,
    /// and no automatic installation fetches a missing one.
    pub fn disable(&self) -> io::Result<()> {
        pin()?;
        self.write_state(false, CISCO_BINARY_LICENSE_SHA256)
    }

    /// Deletes the library and the record of a failed installation and
    /// switches OpenH264 off, so nothing fetches it again until `enable`.
    pub fn remove(&self) -> io::Result<()> {
        let mut paths = vec![self.dir.join(super::codec_runtime_install::RECORD_FILE)];
        paths.extend(self.library_path());
        for path in paths {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        self.disable()
    }

    /// Installs the library from Cisco unless it is present and matches its
    /// pin (`codec_runtime_install.rs`): never while switched off, one
    /// installation at a time, a session not right after a failed attempt.
    pub fn ensure_installed(&self, trigger: Trigger) -> io::Result<PathBuf> {
        self.ensure_with(trigger, &unix_now, &download)
    }

    pub fn ensure_with(
        &self,
        trigger: Trigger,
        clock: &dyn Fn() -> i64,
        fetch: Fetch<'_>,
    ) -> io::Result<PathBuf> {
        let pin = pin()?;
        if self.opted_out() {
            return Err(switched_off());
        }
        let _lock = InstallLock::acquire(&self.dir)?;
        // The user may have switched it off, another installation may have
        // finished or failed while this one waited for the lock.
        if self.opted_out() {
            return Err(switched_off());
        }
        let path = self.dir.join(pin.library_name);
        if verify(&path).is_ok() {
            let _ = clear_record(&self.dir);
            return Ok(path);
        }
        let record = self.record();
        if trigger == Trigger::Session {
            session_gate(record.as_ref(), clock()).map_err(io::Error::other)?;
        }
        if !super::codec_runtime_install::downloads_allowed() {
            return Err(io::Error::other(
                "codec downloads are switched off for this process (SE_CODEC_DOWNLOADS=off)",
            ));
        }
        match fetch(&pin).and_then(|compressed| self.install(&pin, &compressed)) {
            Ok(()) => {
                let _ = clear_record(&self.dir);
                Ok(path)
            }
            Err(error) => {
                let reason = error.to_string();
                eprintln!(
                    "OpenH264 {OPENH264_VERSION}: installation failed ({trigger:?}): {reason}"
                );
                let next = next_record(record.as_ref(), &reason, clock());
                let _ = write_record(&self.dir, &next);
                Err(error)
            }
        }
    }

    /// Whether the OpenH264 coders offer for a session now: present and not
    /// switched off; a store that installs on demand installs it first when
    /// `other_coder` says no other H.264 coder of the session's side serves.
    pub fn session_ready(
        &self,
        other_coder: impl FnOnce() -> bool,
        install: impl FnOnce(&Self) -> io::Result<PathBuf>,
    ) -> bool {
        if self.opted_out() {
            return false;
        }
        if self.library_present() {
            return true;
        }
        if !self.installs_on_demand() || other_coder() {
            return false;
        }
        install(self).is_ok()
    }

    /// The background installer of the daemon: attempts at the start,
    /// repairs a file that does not match its pin, retries on the record's
    /// schedule and keeps watching for a switch back on.
    pub fn install_in_background(&self) {
        let look = |first: bool| Seen {
            opted_out: self.opted_out(),
            installed: if first {
                self.verified()
            } else {
                self.library_present()
            },
            record: self.record(),
        };
        let attempt = || {
            self.ensure_installed(Trigger::Start)
                .map(drop)
                .map_err(|error| error.to_string())
        };
        run_background(true, true, look, attempt);
    }

    /// Checks the compressed file against its pins, unpacks it within
    /// `MAX_LIBRARY_BYTES`, checks the library against its pin and stores
    /// it; nothing is stored on any mismatch.
    fn install(&self, pin: &OpenH264Pin, compressed: &[u8]) -> io::Result<()> {
        let actual = hex_sha256(compressed);
        if compressed.len() as u64 != pin.compressed_size || actual != pin.compressed_sha256 {
            return Err(mismatch("the downloaded OpenH264 file", &actual));
        }
        let mut library = Vec::new();
        bzip2::read::BzDecoder::new(compressed)
            .take(MAX_LIBRARY_BYTES + 1)
            .read_to_end(&mut library)?;
        if library.len() as u64 > MAX_LIBRARY_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the unpacked OpenH264 library exceeds its size limit",
            ));
        }
        let actual = hex_sha256(&library);
        if actual != pin.library_sha256 {
            return Err(mismatch("the unpacked OpenH264 library", &actual));
        }
        crate::support_dirs::write_private_atomic(&self.dir.join(pin.library_name), &library)
    }
}

/// Only the license text this build shows may be named.
fn check_license(accepted: &str) -> io::Result<()> {
    if accepted == CISCO_BINARY_LICENSE_SHA256 {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the Cisco license of OpenH264 was not accepted",
        ))
    }
}

/// Checks `path` against the pin of this platform.
fn verify(path: &Path) -> io::Result<()> {
    let pin = pin()?;
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_LIBRARY_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_LIBRARY_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the OpenH264 file exceeds its size limit",
        ));
    }
    let actual = hex_sha256(&bytes);
    if actual != pin.library_sha256 {
        return Err(mismatch("the OpenH264 file", &actual));
    }
    Ok(())
}

/// Fetches the compressed library from Cisco: HTTPS only, the pinned
/// address without redirects, never a byte beyond the pinned size. No limit
/// on the whole transfer (user rule 2026-11-17): `http_resume` limits the
/// connect and each read and continues a broken transfer with a range
/// request. A first attempt that ends before a byte arrived ends the
/// download (`FirstContact`): a session may wait for it, and the installers
/// retry on their own schedule.
fn download(pin: &OpenH264Pin) -> io::Result<Vec<u8>> {
    if !pin.url.starts_with(DOWNLOAD_HOST) {
        return Err(io::Error::other("OpenH264 address is not Cisco's server"));
    }
    let source = crate::http_resume::Source {
        url: pin.url,
        user_agent: "SmartExplorer-OpenH264/1",
        https_only: true,
        redirects: 0,
        final_url: &|actual| actual == pin.url,
    };
    let bound = crate::http_resume::Bound {
        exact: Some(pin.compressed_size),
        ceiling: None,
    };
    let contact = FirstContact::default();
    let mut bytes = Vec::new();
    let fetched = crate::http_resume::fetch(
        &source,
        bound,
        &mut bytes,
        &mut |kept, _| contact.progress(kept),
        &|| contact.cancelled(),
    );
    match fetched {
        Ok(_) => Ok(bytes),
        Err(_) if contact.gave_up() => Err(io::Error::new(
            io::ErrorKind::NotConnected,
            format!(
                "OpenH264 {OPENH264_VERSION} download: no answer from {DOWNLOAD_HOST} \
                 in the first attempt"
            ),
        )),
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!("OpenH264 {OPENH264_VERSION} download: {error}"),
        )),
    }
}

#[cfg(test)]
#[path = "openh264_install_tests.rs"]
mod tests;
