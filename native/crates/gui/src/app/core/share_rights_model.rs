//! State logic of the compact rights window „Rechte für <Gerät/Raum>“ (plan
//! 2026-10-31 §F, F1–F6, package G2). No egui and no storage in here: the
//! draft of the switches, the presets that fill them, the queue of open
//! confirmations that opens the window, and the two buttons that call the
//! core through a [`RightsBackend`] (`share_rights_core.rs` is the real one,
//! the tests use a recorder).
use crate::share::access_grant::{AccountProfile, Capability, CapabilityDecision};
use crate::share::connection_rights::{
    ConnectionRights, FolderChoice, RightsOrigin, RightsPreset, RightsTarget, MENU_CAPABILITIES,
};
use crate::share::connection_rights_actions::RightsChange;
use crate::share::connection_rights_apply::PendingRightsView;

/// What the window sets the rights of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum RightsSubject {
    /// One device (Direct grant) or one room.
    Connection(RightsTarget),
    /// Every device that connects with this device's Direct code (F5, F6).
    Code,
}

/// The core calls behind „Bestätigen“ and „Abbrechen“.
pub(in crate::app) trait RightsBackend {
    /// Exactly `rights` for `target`; `added` are Direct folders (label,
    /// path) that become exports in the same transaction.
    fn confirm(
        &mut self,
        target: &RightsTarget,
        rights: &ConnectionRights,
        added: &[(String, String)],
    ) -> Result<RightsChange, String>;
    /// The connection stays, without the rights it waited for.
    fn cancel(&mut self, target: &RightsTarget) -> Result<RightsChange, String>;
    /// The rights of the Direct code (`None`: devices wait for them).
    fn set_code_rights(
        &mut self,
        rights: Option<&ConnectionRights>,
        added: &[(String, String)],
    ) -> Result<RightsChange, String>;
}

/// How long commands, terminal and screen last (files are not bound to it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum ExpiryChoice {
    Unlimited,
    /// The end the rights already have (unix seconds).
    Keep(i64),
    Hour,
    Day,
    Week,
}

impl ExpiryChoice {
    pub(in crate::app) const FIXED: [ExpiryChoice; 4] =
        [Self::Unlimited, Self::Hour, Self::Day, Self::Week];

    pub(in crate::app) fn of(valid_until: Option<i64>) -> Self {
        valid_until.map_or(Self::Unlimited, Self::Keep)
    }

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Unlimited => "Unbegrenzt",
            Self::Keep(_) => "Bisherige Dauer",
            Self::Hour => "1 Stunde",
            Self::Day => "1 Tag",
            Self::Week => "7 Tage",
        }
    }

    /// The end as unix seconds of the host clock, counted from `now`.
    pub(in crate::app) fn resolve(self, now: i64) -> Option<i64> {
        match self {
            Self::Unlimited => None,
            Self::Keep(end) => Some(end),
            Self::Hour => Some(now.saturating_add(3_600)),
            Self::Day => Some(now.saturating_add(86_400)),
            Self::Week => Some(now.saturating_add(7 * 86_400)),
        }
    }
}

/// The three ways to choose the folders (F2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum FolderMode {
    All,
    None,
    Selected,
}

/// The switches of the window, filled by a preset or by hand.
#[derive(Clone, Debug)]
pub(in crate::app) struct RightsDraft {
    pub(in crate::app) subject: RightsSubject,
    pub(in crate::app) name: String,
    /// `Some`: the connection waits for these rights („Rechte offen“).
    pub(in crate::app) origin: Option<RightsOrigin>,
    pub(in crate::app) rights: ConnectionRights,
    pub(in crate::app) expiry: ExpiryChoice,
    /// Folders picked with „Ordner hinzufügen“ (label, path); they become
    /// Direct exports only with the confirmation.
    pub(in crate::app) added_folders: Vec<(String, String)>,
}

impl RightsDraft {
    pub(in crate::app) fn for_pending(view: &PendingRightsView) -> Self {
        let mut draft = Self::new(
            RightsSubject::Connection(view.target.clone()),
            &view.name,
            view.rights.clone(),
        );
        draft.origin = Some(view.origin);
        draft
    }

    pub(in crate::app) fn new(
        subject: RightsSubject,
        name: &str,
        rights: ConnectionRights,
    ) -> Self {
        Self {
            subject,
            name: name.to_string(),
            origin: None,
            expiry: ExpiryChoice::of(rights.valid_until),
            rights,
            added_folders: Vec::new(),
        }
    }

    pub(in crate::app) fn for_code(rights: Option<ConnectionRights>) -> Self {
        Self::new(
            RightsSubject::Code,
            "Geräte mit meinem Direkt-Code",
            rights.unwrap_or_default(),
        )
    }

    /// F4: the preset fills the switches; account and duration stay, every
    /// switch stays changeable.
    pub(in crate::app) fn apply_preset(&mut self, preset: RightsPreset) {
        self.rights = ConnectionRights {
            account: self.rights.account.clone(),
            valid_until: self.rights.valid_until,
            ..preset.rights()
        };
    }

    /// The preset these switches are exactly (the duration is not part of one).
    pub(in crate::app) fn selected_preset(&self) -> Option<RightsPreset> {
        let mut rights = self.rights.clone();
        rights.valid_until = None;
        rights.preset()
    }

    pub(in crate::app) fn set_decision(
        &mut self,
        capability: Capability,
        decision: CapabilityDecision,
    ) {
        self.rights.set_decision(capability, decision);
    }

    pub(in crate::app) fn set_account(&mut self, account: AccountProfile) {
        self.rights.account = account;
    }

    /// Whether the draft is for a room (its „Alle“ is a copy, G5).
    pub(in crate::app) fn is_room(&self) -> bool {
        matches!(
            self.subject,
            RightsSubject::Connection(RightsTarget::Room { .. })
        )
    }

    pub(in crate::app) fn folder_mode(&self) -> FolderMode {
        match self.rights.folders {
            FolderChoice::All => FolderMode::All,
            FolderChoice::None => FolderMode::None,
            FolderChoice::Selected { .. } => FolderMode::Selected,
        }
    }

    /// Switches the way the folders are chosen; „Auswahl“ starts with every
    /// folder in `available` (the Direct folders and the added ones) ticked.
    pub(in crate::app) fn set_folder_mode(&mut self, mode: FolderMode, available: &[String]) {
        if mode == self.folder_mode() {
            return;
        }
        self.rights.folders = match mode {
            FolderMode::All => FolderChoice::All,
            FolderMode::None => FolderChoice::None,
            FolderMode::Selected => FolderChoice::Selected {
                paths: available.to_vec(),
                connections: false,
            },
        };
    }

    pub(in crate::app) fn folder_ticked(&self, path: &str) -> bool {
        matches!(&self.rights.folders, FolderChoice::Selected { paths, .. }
            if paths.iter().any(|ticked| ticked == path))
    }

    pub(in crate::app) fn connections_ticked(&self) -> bool {
        matches!(
            self.rights.folders,
            FolderChoice::Selected {
                connections: true,
                ..
            }
        )
    }

    /// Ticks or unticks one folder; only meaningful in mode „Auswahl“.
    pub(in crate::app) fn tick_folder(&mut self, path: &str, on: bool) {
        if let FolderChoice::Selected { paths, .. } = &mut self.rights.folders {
            paths.retain(|ticked| ticked != path);
            if on {
                paths.push(path.to_string());
            }
        }
    }

    pub(in crate::app) fn tick_connections(&mut self, on: bool) {
        if let FolderChoice::Selected { connections, .. } = &mut self.rights.folders {
            *connections = on;
        }
    }

    /// A folder picked with „Ordner hinzufügen“: part of the selection from
    /// now on (mode „Auswahl“), exported with the confirmation.
    pub(in crate::app) fn add_folder(&mut self, label: &str, path: &str, available: &[String]) {
        let path = path.trim().replace('\\', "/");
        if path.is_empty() {
            return;
        }
        if !self.added_folders.iter().any(|(_, known)| known == &path) {
            self.added_folders.push((label.to_string(), path.clone()));
        }
        let mut all = available.to_vec();
        if !all.contains(&path) {
            all.push(path.clone());
        }
        self.set_folder_mode(FolderMode::Selected, &all);
        self.tick_folder(&path, true);
    }

    /// The rights this draft confirms, with the duration counted from `now`;
    /// an empty selection counts as „Keine Ordner“.
    pub(in crate::app) fn finish(&self, now: i64) -> Result<ConnectionRights, String> {
        let mut rights = self.rights.clone();
        rights.valid_until = self.expiry.resolve(now);
        if let FolderChoice::Selected { paths, connections } = &rights.folders {
            if paths.is_empty() && !connections {
                rights.folders = FolderChoice::None;
            }
        }
        rights.validate()?;
        Ok(rights)
    }

    /// „Bestätigen“: exactly the switches of this draft, through the core.
    pub(in crate::app) fn confirm(
        &self,
        backend: &mut dyn RightsBackend,
        now: i64,
    ) -> Result<RightsChange, String> {
        let rights = self.finish(now)?;
        match &self.subject {
            RightsSubject::Connection(target) => {
                backend.confirm(target, &rights, &self.added_folders)
            }
            RightsSubject::Code => backend.set_code_rights(Some(&rights), &self.added_folders),
        }
    }

    /// „Abbrechen“: a connection that waited keeps existing without rights;
    /// a draft that did not wait (or the code) just closes. `None` then.
    pub(in crate::app) fn cancel(
        &self,
        backend: &mut dyn RightsBackend,
    ) -> Result<Option<RightsChange>, String> {
        match (&self.subject, self.origin) {
            (RightsSubject::Connection(target), Some(_)) => backend.cancel(target).map(Some),
            _ => Ok(None),
        }
    }
}

/// How long a settled confirmation is ignored when an older worker snapshot
/// still lists it as open.
const SETTLED_IGNORE: std::time::Duration = std::time::Duration::from_secs(5);

/// The window's memory: the open draft, the confirmations the user put off
/// with the close button (they stay „Rechte offen“ on the device page) and
/// the ones just confirmed or cancelled.
#[derive(Clone, Debug, Default)]
pub(in crate::app) struct RightsWindowState {
    pub(in crate::app) draft: Option<RightsDraft>,
    pub(in crate::app) dismissed: Vec<RightsTarget>,
    pub(in crate::app) settled: Vec<(RightsTarget, std::time::Instant)>,
    pub(in crate::app) error: Option<String>,
}

impl RightsWindowState {
    /// Called every frame with the open confirmations: opens the first one
    /// that was not put off, drops a draft whose confirmation was settled
    /// elsewhere (CLI, other window), and forgets put-off ones that are gone.
    pub(in crate::app) fn sync(&mut self, pending: &[PendingRightsView], now: std::time::Instant) {
        self.settled
            .retain(|(_, at)| now.duration_since(*at) < SETTLED_IGNORE);
        let pending: Vec<&PendingRightsView> = pending
            .iter()
            .filter(|view| {
                !self
                    .settled
                    .iter()
                    .any(|(target, _)| target == &view.target)
            })
            .collect();
        self.dismissed
            .retain(|target| pending.iter().any(|view| &view.target == target));
        if let Some(draft) = &self.draft {
            if let (RightsSubject::Connection(target), Some(_)) = (&draft.subject, draft.origin) {
                if !pending.iter().any(|view| &view.target == target) {
                    self.draft = None;
                    self.error = None;
                }
            }
        }
        if self.draft.is_none() {
            if let Some(view) = pending
                .iter()
                .find(|view| !self.dismissed.contains(&view.target))
            {
                self.draft = Some(RightsDraft::for_pending(view));
                self.error = None;
            }
        }
    }

    /// Opens `draft` (device page, code row), replacing an open one.
    pub(in crate::app) fn open(&mut self, draft: RightsDraft) {
        self.draft = Some(draft);
        self.error = None;
    }

    /// The close button: the confirmation stays open, the window does not
    /// come back by itself.
    pub(in crate::app) fn later(&mut self) {
        if let Some(draft) = self.draft.take() {
            if let (RightsSubject::Connection(target), Some(_)) = (draft.subject, draft.origin) {
                if !self.dismissed.contains(&target) {
                    self.dismissed.push(target);
                }
            }
        }
        self.error = None;
    }

    /// „Bestätigen“ or „Abbrechen“ went through: the window closes and an
    /// older snapshot that still lists the connection as open does not
    /// bring it back.
    pub(in crate::app) fn settled(&mut self, draft: &RightsDraft, now: std::time::Instant) {
        if let RightsSubject::Connection(target) = &draft.subject {
            self.settled.push((target.clone(), now));
        }
        self.draft = None;
        self.error = None;
    }
}

/// Where a connection waiting for rights came from, in the window's words.
pub(in crate::app) fn origin_text(origin: RightsOrigin) -> &'static str {
    match origin {
        RightsOrigin::Request => "Anfrage angenommen",
        RightsOrigin::Pairing => "Per PIN gekoppelt",
        RightsOrigin::ShareBack => "Rückfreigabe geöffnet",
        RightsOrigin::Repair => "Verbindung ergänzt",
        RightsOrigin::IdentityReplaced => "Das Gerät hat einen neuen Schlüssel",
        RightsOrigin::Readmitted => "Freigabe wieder erlaubt",
        RightsOrigin::RoomCreated => "Raum erstellt",
        RightsOrigin::RoomJoined => "Raum beigetreten",
    }
}

/// F6: what automatic acceptance does, for the warning on the page „Anfragen“.
pub(in crate::app) fn auto_accept_text(code_rights: Option<&ConnectionRights>) -> String {
    let tail = "Entfernte Identitäten bleiben gesperrt.";
    match code_rights.filter(|rights| rights.grants_anything()) {
        None => format!(
            "Geräte mit deinem Direkt-Code werden ohne deine Bestätigung verbunden, haben aber \
             zunächst keine Rechte: kein Ordner, kein Schreiben, keine Befehle, kein Bildschirm. \
             Du legst sie danach im Fenster „Rechte“ fest. {tail}"
        ),
        Some(rights) => format!(
            "Geräte mit deinem Direkt-Code werden ohne deine Bestätigung verbunden und erhalten \
             sofort: {}. {tail}",
            rights.summary()
        ),
    }
}

/// One line for the Direct code: what a device that connects with it gets.
pub(in crate::app) fn code_rights_text(code_rights: Option<&ConnectionRights>) -> String {
    match code_rights {
        None => "Wird beim Verbinden erfragt: das Gerät wartet ohne Rechte, bis du sie bestätigst."
            .to_string(),
        Some(rights) if !rights.grants_anything() => {
            "Dieser Code vergibt keine Rechte: Geräte sind verbunden, mehr nicht.".to_string()
        }
        Some(rights) => format!("Dieser Code vergibt: {}.", rights.summary()),
    }
}

/// The capabilities the window shows as rows, with their labels.
pub(in crate::app) const DECISION_ROWS: [(Capability, &str, &str); 4] = [
    (
        Capability::ExecCommands,
        "Befehle",
        "Das Gerät führt Befehle als das gewählte Konto aus.",
    ),
    (
        Capability::ExecTerminal,
        "Terminal",
        "Das Gerät öffnet eine Shell als das gewählte Konto.",
    ),
    (
        Capability::ScreenView,
        "Desktop sehen",
        "Das Gerät sieht Bildschirm und Fenster.",
    ),
    (
        Capability::ScreenControl,
        "Desktop steuern",
        "Erlauben schließt Programme, Anmeldung, lokale Dienste und den eigenen Remote Desktop dieses Geräts ein.",
    ),
];

const _: () = assert!(DECISION_ROWS.len() == MENU_CAPABILITIES.len());

#[cfg(test)]
#[path = "share_rights_task_tests.rs"]
mod abgleich_task_tests;
