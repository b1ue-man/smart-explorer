//! The OpenH264 library of this user (SCR-M3, Cisco conditions 1 to 3): the
//! user downloads it from Cisco after reading the license (`enable`), may
//! switch it off and on again (`disable`, `enable`) and remove it. It lives
//! in the private app data folder `screen/openh264/` beside a state file. A
//! download is HTTPS only, from the pinned address without redirects, and
//! is accepted only with the pinned size and SHA-256; the unpacked library
//! must match its pin as well. Every load checks the file against its pin
//! right before (`verified_library`), so a changed file is never loaded.
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::openh264_pins::{
    OpenH264Pin, CISCO_BINARY_LICENSE_SHA256, DOWNLOAD_HOST, MAX_LIBRARY_BYTES, PIN,
};
use crate::screen::core::codec_registry::{OpenH264Report, OpenH264State, OPENH264_VERSION};

const STATE_FILE: &str = "state.json";
const MAX_STATE_BYTES: u64 = 4096;
/// Time for the download of about 0.6 MB.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);

/// `state.json` of the store.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct StoredState {
    enabled: bool,
    version: String,
    /// The license text the user accepted (its SHA-256).
    license_sha256: String,
    /// Unix seconds of the last switch.
    changed_at: i64,
}

/// The folder with the library and its state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OpenH264Store {
    dir: PathBuf,
}

pub(crate) fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
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

impl OpenH264Store {
    /// The store of this user: `<app data>/screen/openh264`.
    pub(crate) fn for_user() -> Self {
        let data = crate::support_dirs::app_data_dir();
        Self::at(data.join("screen").join("openh264"))
    }

    pub(crate) fn at(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Where the library lies; `None` where no pin fits the platform.
    pub(crate) fn library_path(&self) -> Option<PathBuf> {
        PIN.map(|pin| self.dir.join(pin.library_name))
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join(STATE_FILE)
    }

    fn read_state(&self) -> Option<StoredState> {
        let path = self.state_path();
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

    /// Switched on by the user and present; the pin is checked at load.
    pub(crate) fn switched_on(&self) -> bool {
        let present = self.library_path().is_some_and(|path| path.is_file());
        present && self.read_state().is_some_and(|state| state.enabled)
    }

    /// The state for `se screen codec` and the codec view, with the pin
    /// check of the file.
    pub(crate) fn report(&self) -> OpenH264Report {
        let Some(path) = self.library_path() else {
            let reason = "OpenH264 2.6.0 is not offered for this platform";
            return OpenH264Report::new(OpenH264State::Absent, Some(reason.into()));
        };
        if !path.is_file() {
            return OpenH264Report::new(OpenH264State::Absent, None);
        }
        if let Err(error) = verify(&path) {
            return OpenH264Report::new(OpenH264State::Failed, Some(error.to_string()));
        }
        let state = match self.read_state() {
            Some(state) if state.enabled => OpenH264State::Enabled,
            _ => OpenH264State::Disabled,
        };
        OpenH264Report::new(state, None)
    }

    /// The library after its SHA-256 matched the pin; only a library the
    /// user switched on is handed out.
    pub(crate) fn verified_library(&self) -> io::Result<PathBuf> {
        let pin = pin()?;
        if !self.read_state().is_some_and(|state| state.enabled) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "OpenH264 is switched off",
            ));
        }
        let path = self.dir.join(pin.library_name);
        verify(&path)?;
        Ok(path)
    }

    /// Switches OpenH264 on after the user accepted the license whose
    /// SHA-256 is `accepted`; downloads the library from Cisco when it is
    /// missing or does not match its pin.
    pub(crate) fn enable(&self, accepted: &str) -> io::Result<()> {
        check_license(accepted)?;
        let pin = pin()?;
        if verify(&self.dir.join(pin.library_name)).is_err() {
            let compressed = download(&pin)?;
            self.install(&pin, &compressed)?;
        }
        self.write_state(true, accepted)
    }

    /// The same with the compressed file fetched elsewhere (the task suite,
    /// an offline copy): it must match the pinned size and SHA-256 as well.
    pub(crate) fn enable_from_compressed(
        &self,
        accepted: &str,
        compressed: &[u8],
    ) -> io::Result<()> {
        check_license(accepted)?;
        let pin = pin()?;
        self.install(&pin, compressed)?;
        self.write_state(true, accepted)
    }

    /// Switches OpenH264 off; the file stays for switching on again.
    pub(crate) fn disable(&self) -> io::Result<()> {
        let state = self
            .read_state()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "OpenH264 is not downloaded"))?;
        self.write_state(false, &state.license_sha256)
    }

    /// Deletes the library and its state.
    pub(crate) fn remove(&self) -> io::Result<()> {
        let mut paths = vec![self.state_path()];
        paths.extend(self.library_path());
        for path in paths {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
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

/// Only the license text this build shows may be accepted.
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
/// address without redirects, at most the pinned size plus one byte.
fn download(pin: &OpenH264Pin) -> io::Result<Vec<u8>> {
    if !pin.url.starts_with(DOWNLOAD_HOST) {
        return Err(io::Error::other("OpenH264 address is not Cisco's server"));
    }
    let response = ureq::AgentBuilder::new()
        .https_only(true)
        .redirects(0)
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .get(pin.url)
        .set("User-Agent", "SmartExplorer-OpenH264/1")
        .call()
        .map_err(|error| {
            io::Error::other(format!("OpenH264 {OPENH264_VERSION} download: {error}"))
        })?;
    if response.status() != 200 || response.get_url() != pin.url {
        return Err(io::Error::other(format!(
            "OpenH264 download ended at {} with status {}",
            response.get_url(),
            response.status()
        )));
    }
    if let Some(length) = response.header("Content-Length") {
        if length.trim().parse::<u64>().ok() != Some(pin.compressed_size) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "OpenH264 download announces {length} bytes, the pin {}",
                    pin.compressed_size
                ),
            ));
        }
    }
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(pin.compressed_size + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
#[path = "openh264_install_tests.rs"]
mod tests;
