//! Viewer side of `login_target_v1` (E22): the targets paired devices sent,
//! bound to the device and its Share key, the choice between the device
//! form and the explicit file form of „Anmeldung öffnen“, and the automatic
//! switch between screen view and login view.
//!
//! The book lives in `<sync data>/login-targets.json`. Only an accepted
//! message of the pinned session changes it ([`LoginTargetBook::apply`]); a
//! withdrawal (the host revoked steering, removed its gateway or the grant)
//! deletes the device's target, so does removing the contact.
use super::target_wire::{
    accept, Accepted, LoginTargetMessage, Refusal, SessionPeer, WithdrawReason,
};
use crate::login::core::gateway_types::{GatewayTarget, MAX_NETWORK};
use serde::{Deserialize, Serialize};
use std::{
    fmt, io,
    path::{Path, PathBuf},
    sync::Mutex,
};

pub const STORE_FILE: &str = "login-targets.json";
const STORE_FORMAT: u32 = 1;
/// As many paired hosts as a login policy has grants.
pub const MAX_DEVICES: usize = 256;
const MAX_FILE: usize = MAX_DEVICES * (MAX_NETWORK + 4096);
/// One writer at a time inside a process (worker, Android app).
static WRITE: Mutex<()> = Mutex::new(());

/// The target one paired device sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairedLoginTarget {
    pub device_id: String,
    pub public_key: String,
    pub node_id: String,
    pub device_name: String,
    pub relation_id: String,
    pub target_user: String,
    pub target: GatewayTarget,
    pub received_at: u64,
}

/// A paired device whose Share version has no `login_target_v1`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyPeer {
    pub device_id: String,
    pub public_key: String,
    pub device_name: String,
    pub relation_id: String,
    pub noted_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginTargetBook {
    format: u32,
    #[serde(default)]
    targets: Vec<PairedLoginTarget>,
    #[serde(default)]
    legacy: Vec<LegacyPeer>,
}

impl Default for LoginTargetBook {
    fn default() -> Self {
        Self {
            format: STORE_FORMAT,
            targets: Vec::new(),
            legacy: Vec::new(),
        }
    }
}

/// What [`LoginTargetBook::apply`] changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Stored,
    Removed(WithdrawReason),
    Unchanged,
}

/// A failed lookup, with the same code and text on every surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookupError {
    pub code: &'static str,
    pub message: String,
}

impl fmt::Display for LookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl LoginTargetBook {
    pub fn targets(&self) -> &[PairedLoginTarget] {
        &self.targets
    }

    pub fn legacy(&self) -> &[LegacyPeer] {
        &self.legacy
    }

    /// Applies one host message after [`accept`]; a refused message changes
    /// nothing.
    pub fn apply(
        &mut self,
        peer: &SessionPeer,
        message: &LoginTargetMessage,
        now: u64,
    ) -> Result<Change, Refusal> {
        match accept(peer, message)? {
            Accepted::Store {
                target_user,
                target,
            } => {
                self.legacy.retain(|old| old.device_id != peer.device_id);
                let entry = PairedLoginTarget {
                    device_id: peer.device_id.clone(),
                    public_key: peer.public_key.clone(),
                    node_id: peer.node_id.clone(),
                    device_name: peer.device_name.clone(),
                    relation_id: peer.relation_id.clone(),
                    target_user,
                    target,
                    received_at: now,
                };
                if let Some(old) = self
                    .targets
                    .iter_mut()
                    .find(|old| old.device_id == entry.device_id)
                {
                    let same = PairedLoginTarget {
                        received_at: old.received_at,
                        ..entry.clone()
                    } == *old;
                    *old = entry;
                    return Ok(if same {
                        Change::Unchanged
                    } else {
                        Change::Stored
                    });
                }
                if self.targets.len() >= MAX_DEVICES {
                    // The longest unrenewed target makes room.
                    if let Some(oldest) = self
                        .targets
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, old)| old.received_at)
                        .map(|(index, _)| index)
                    {
                        self.targets.remove(oldest);
                    }
                }
                self.targets.push(entry);
                Ok(Change::Stored)
            }
            Accepted::Remove(reason) => Ok(if self.forget(&peer.device_id) {
                Change::Removed(reason)
            } else {
                Change::Unchanged
            }),
        }
    }

    /// The pinned session's host has no `login_target_v1`: a target it sent
    /// before (newer version) is dropped, the device is noted as legacy;
    /// `true` when the book changed.
    pub fn note_legacy(&mut self, peer: &SessionPeer, now: u64) -> bool {
        if !peer.pinned {
            return false;
        }
        self.forget(&peer.device_id);
        if self.legacy.len() >= MAX_DEVICES {
            self.legacy.remove(0);
        }
        self.legacy.push(LegacyPeer {
            device_id: peer.device_id.clone(),
            public_key: peer.public_key.clone(),
            device_name: peer.device_name.clone(),
            relation_id: peer.relation_id.clone(),
            noted_at: now,
        });
        true
    }

    /// Removes everything about `device_id`; `true` when a target was there.
    pub fn forget(&mut self, device_id: &str) -> bool {
        let before = self.targets.len();
        self.targets.retain(|old| old.device_id != device_id);
        self.legacy.retain(|old| old.device_id != device_id);
        before != self.targets.len()
    }

    /// Keeps only devices that are still contacts with the same key.
    pub fn retain(&mut self, keep: impl Fn(&str, &str) -> bool) -> bool {
        let before = (self.targets.len(), self.legacy.len());
        self.targets
            .retain(|old| keep(&old.device_id, &old.public_key));
        self.legacy
            .retain(|old| keep(&old.device_id, &old.public_key));
        before != (self.targets.len(), self.legacy.len())
    }

    /// The target of exactly this device and Share key (device page).
    pub fn bound(&self, device_id: &str, public_key: &str) -> Option<&PairedLoginTarget> {
        self.targets
            .iter()
            .find(|old| old.device_id == device_id && old.public_key == public_key)
    }

    /// The target `selector` (device ID, contact or room ID, or a unique
    /// device name) names.
    pub fn resolve(&self, selector: &str) -> Result<PairedLoginTarget, LookupError> {
        let selector = selector.trim();
        let matches = |id: &str, relation: &str, name: &str| {
            id == selector || relation == selector || name.eq_ignore_ascii_case(selector)
        };
        let found: Vec<_> = self
            .targets
            .iter()
            .filter(|old| matches(&old.device_id, &old.relation_id, &old.device_name))
            .collect();
        match found.as_slice() {
            [one] => return Ok((*one).clone()),
            [] => {}
            many => {
                let names: Vec<String> = many
                    .iter()
                    .map(|old| format!("{} ({})", old.device_name, old.device_id))
                    .collect();
                return Err(LookupError {
                    code: "login_target_ambiguous",
                    message: format!(
                        "„{selector}“ passt auf mehrere Geräte: {}. Bitte die Geräte-ID angeben.",
                        names.join(", ")
                    ),
                });
            }
        }
        if let Some(old) = self
            .legacy
            .iter()
            .find(|old| matches(&old.device_id, &old.relation_id, &old.device_name))
        {
            return Err(LookupError {
                code: "login_target_legacy_peer",
                message: format!(
                    "„{}“ kennt login_target_v1 nicht (ältere Smart-Explorer-Version) und sendet kein Anmeldeziel. Mit einer aktuellen Version dort genügt „Bildschirm steuern: Erlauben“; bis dahin öffnet nur ein Maschinenziel als Datei mit --acknowledge-machine-pin.",
                    old.device_name
                ),
            });
        }
        Err(missing(selector))
    }

    /// Reads the book; a missing file is an empty book, an entry that no
    /// longer validates is left out.
    pub fn load_from(path: &Path) -> io::Result<Self> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        if bytes.len() > MAX_FILE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Anmeldeziele überschreiten ihre Größe",
            ));
        }
        let mut book: Self = serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if book.format != STORE_FORMAT {
            return Ok(Self::default());
        }
        book.targets
            .retain(|old| old.target.validate().is_ok() && old.target.public_key != old.public_key);
        book.targets.truncate(MAX_DEVICES);
        book.legacy.truncate(MAX_DEVICES);
        Ok(book)
    }

    /// Writes the book through a staged file and a rename.
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        let bytes = serde_json::to_vec(self).map_err(io::Error::other)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let staged = path.with_extension("json.tmp");
        std::fs::write(&staged, bytes)?;
        std::fs::rename(&staged, path)
    }
}

fn missing(selector: &str) -> LookupError {
    LookupError {
        code: "login_target_missing",
        message: format!(
            "Für „{selector}“ liegt kein Anmeldeziel vor. Das Gerät sendet es selbst über die gekoppelte Verbindung, sobald es diesem Gerät „Bildschirm steuern: Erlauben“ gegeben hat und seine Maschinenanmeldung eingerichtet ist."
        ),
    }
}

/// `<sync data>/login-targets.json`.
pub fn store_path() -> PathBuf {
    crate::support_dirs::sync_data_dir().join(STORE_FILE)
}

/// Reads the book of this device.
pub fn load() -> io::Result<LoginTargetBook> {
    LoginTargetBook::load_from(&store_path())
}

/// Changes the book of this device under the process writer lock; it is
/// written only when `change` reports a change.
pub fn update<T>(change: impl FnOnce(&mut LoginTargetBook) -> (T, bool)) -> io::Result<T> {
    let _write = WRITE
        .lock()
        .map_err(|_| io::Error::other("Anmeldeziele sind gesperrt"))?;
    let path = store_path();
    let mut book = LoginTargetBook::load_from(&path)?;
    let (result, changed) = change(&mut book);
    if changed {
        book.save_to(&path)?;
    }
    Ok(result)
}

/// Which form „Anmeldung öffnen“ takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenChoice {
    /// The explicit administrative file form (`TARGET_FILE`,
    /// `--target-user`, `--acknowledge-machine-pin`), unchanged.
    File,
    /// The target the paired device sent.
    Device(PairedLoginTarget),
}

/// The explicit pin acknowledgement keeps the file form; without it the
/// selector names a paired device. A selector that names no paired device
/// but an existing file keeps the old answer of the file form.
pub fn choose_open(
    book: &LoginTargetBook,
    selector: &str,
    acknowledged: bool,
    file_exists: bool,
) -> Result<OpenChoice, LookupError> {
    if acknowledged {
        return Ok(OpenChoice::File);
    }
    match book.resolve(selector) {
        Ok(target) => Ok(OpenChoice::Device(target)),
        Err(error) if error.code == "login_target_missing" && file_exists => Err(LookupError {
            code: "machine_pin_acknowledgement_required",
            message: "explicit acknowledgement of the administrative machine pin is required"
                .into(),
        }),
        Err(error) => Err(error),
    }
}

/// What the viewer shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewerSurface {
    #[default]
    Screen,
    Login,
}

/// What the viewer does after an observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwitchAction {
    Stay,
    /// Release held inputs of the screen view, then open the login view.
    EnterLogin,
    /// Close the login view (it releases its inputs), show the screen again.
    LeaveLogin,
}

/// The screen states in which the host shows its secure desktop, its lock
/// screen or no user session (`SessionState::key`).
pub fn login_state(screen_state: &str) -> bool {
    matches!(
        screen_state,
        "secure_desktop" | "host_locked" | "session_inactive"
    )
}

/// The automatic switch (E22): no click. It enters the login view while the
/// stream pauses with a login state and a paired target exists, and returns
/// as soon as the stream shows the user session again. A login view that
/// failed in one pause is not reopened before the pause ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoginSwitch {
    surface: ViewerSurface,
    failed_in_pause: bool,
}

impl LoginSwitch {
    pub fn surface(&self) -> ViewerSurface {
        self.surface
    }

    pub fn observe(&mut self, screen_state: &str, target_known: bool) -> SwitchAction {
        let paused = login_state(screen_state);
        if !paused {
            self.failed_in_pause = false;
        }
        match self.surface {
            ViewerSurface::Screen if paused && target_known && !self.failed_in_pause => {
                self.surface = ViewerSurface::Login;
                SwitchAction::EnterLogin
            }
            ViewerSurface::Login if screen_state == "streaming" => {
                self.surface = ViewerSurface::Screen;
                SwitchAction::LeaveLogin
            }
            _ => SwitchAction::Stay,
        }
    }

    /// The login view ended or could not open during this pause.
    pub fn login_failed(&mut self) -> SwitchAction {
        self.failed_in_pause = true;
        if self.surface == ViewerSurface::Login {
            self.surface = ViewerSurface::Screen;
            SwitchAction::LeaveLogin
        } else {
            SwitchAction::Stay
        }
    }
}
