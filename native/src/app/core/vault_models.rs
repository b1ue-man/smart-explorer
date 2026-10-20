//! Calculation models of the vault interface of the desktop (strand S3,
//! VLT-M10, U29): the rows of the sidebar section „Tresore“, the lock events
//! and their effect on tabs, the outcome texts of unlocking and locking, the
//! open rule for vault files and the recovery-key page. No `egui` and no
//! input/output here: the windows only draw what these functions return, and
//! the unit tests (`vault_models_tests.rs`) check them without a window.
#![allow(dead_code)] // some accessors are the acceptance signals (tests) and the surfaces of later packages
use std::collections::HashMap;
use std::path::PathBuf;

/// The state of one vault site in the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowState {
    Locked,
    Unlocked,
    Locking,
    Unavailable,
}

impl RowState {
    /// The wire name of the worker (`locked`, `unlocked`, `locking`,
    /// `unavailable`); a name this build does not know counts as locked.
    pub(crate) fn from_wire(text: &str) -> Self {
        match text {
            "unlocked" => Self::Unlocked,
            "locking" => Self::Locking,
            "unavailable" => Self::Unavailable,
            _ => Self::Locked,
        }
    }

    /// The mark after the label.
    pub(crate) fn icon(self) -> &'static str {
        match self {
            Self::Locked => "🔒",
            Self::Unlocked => "🔓",
            Self::Locking => "⏳",
            Self::Unavailable => "⚠",
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Locked => "Gesperrt",
            Self::Unlocked => "Entsperrt",
            Self::Locking => "Wird gesperrt …",
            Self::Unavailable => "Nicht erreichbar",
        }
    }
}

/// The text of a state: `Nicht erreichbar` carries its reason.
pub(crate) fn row_text(state: RowState, reason: Option<&str>) -> String {
    match (
        state,
        reason.map(str::trim).filter(|reason| !reason.is_empty()),
    ) {
        (RowState::Unavailable, Some(reason)) => format!("{}: {reason}", state.title()),
        _ => state.title().to_string(),
    }
}

/// The hover text of a sidebar row.
pub(crate) fn row_hover(label: &str, state: RowState, reason: Option<&str>) -> String {
    format!("{label} – {}", row_text(state, reason))
}

/// One site as the worker reports it (`Vault::Status`), without the wire
/// type: the models stay testable on their own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SiteModel {
    pub(crate) site: String,
    pub(crate) label: String,
    pub(crate) state: String,
    pub(crate) reason: Option<String>,
    pub(crate) remember_password: bool,
    pub(crate) shortening_threshold: Option<u32>,
}

/// One change of a site (`VaultEvent`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EventModel {
    pub(crate) seq: u64,
    pub(crate) site: String,
    pub(crate) state: String,
    pub(crate) reason: Option<String>,
}

impl EventModel {
    /// Whether the event ends the access to the site.
    pub(crate) fn is_lock(&self) -> bool {
        matches!(self.state.as_str(), "locked" | "locking")
    }
}

/// The answer of `Vault::Status`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct StatusModel {
    pub(crate) sites: Vec<SiteModel>,
    pub(crate) event_seq: u64,
    pub(crate) events: Vec<EventModel>,
    /// Which lock signals the worker receives on this system (T14).
    pub(crate) sleep_lock: bool,
    pub(crate) screen_lock: bool,
    /// The note next to the screen lock switch (K13).
    pub(crate) screen_lock_note: Option<String>,
}

/// One entry of the vault list, read from its file (`SiteRecord`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RecordModel {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) location: String,
    pub(crate) vault_id: String,
    pub(crate) replica_of: Option<String>,
    pub(crate) remember_password: bool,
    pub(crate) auto_lock_idle_min: Option<u32>,
    pub(crate) lock_on_sleep: bool,
    pub(crate) lock_on_screen_lock: bool,
}

impl RecordModel {
    /// The label, or the ID when the label is empty.
    pub(crate) fn display_label(&self) -> &str {
        if self.label.trim().is_empty() {
            &self.id
        } else {
            &self.label
        }
    }
}

/// One line of the sidebar section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VaultRow {
    pub(crate) site: String,
    pub(crate) label: String,
    pub(crate) state: RowState,
    pub(crate) reason: Option<String>,
    pub(crate) record: RecordModel,
}

/// The rows: every entered site in the order of the list, with the state of
/// the worker; without a worker (`None`) every site is locked (V10). A site
/// only the worker names (a broken list entry) comes last.
pub(crate) fn rows(status: Option<&StatusModel>, records: &[RecordModel]) -> Vec<VaultRow> {
    let of = |site: &str| status.and_then(|status| status.sites.iter().find(|s| s.site == site));
    let mut out: Vec<VaultRow> = records
        .iter()
        .map(|record| {
            let known = of(&record.id);
            VaultRow {
                site: record.id.clone(),
                label: record.display_label().to_string(),
                state: known.map_or(RowState::Locked, |site| RowState::from_wire(&site.state)),
                reason: known.and_then(|site| site.reason.clone()),
                record: record.clone(),
            }
        })
        .collect();
    if let Some(status) = status {
        for site in &status.sites {
            if records.iter().any(|record| record.id == site.site) {
                continue;
            }
            let label = if site.label.trim().is_empty() {
                site.site.clone()
            } else {
                site.label.clone()
            };
            out.push(VaultRow {
                site: site.site.clone(),
                label,
                state: RowState::from_wire(&site.state),
                reason: site.reason.clone(),
                record: RecordModel {
                    id: site.site.clone(),
                    label: site.label.clone(),
                    remember_password: site.remember_password,
                    ..RecordModel::default()
                },
            });
        }
    }
    out
}

/// What the `vault-feed` thread keeps (U29-2): the last status, the vault
/// list, the last error, the newest event seen and the states before.
#[derive(Clone, Debug, Default)]
pub(crate) struct VaultFeedState {
    pub(crate) status: Option<StatusModel>,
    pub(crate) records: Vec<RecordModel>,
    pub(crate) error: Option<String>,
    pub(crate) seen_seq: u64,
    previous: HashMap<String, RowState>,
}

impl VaultFeedState {
    /// Takes a poll. A failed status request keeps the last status (a
    /// hiccup of the connection is no lock); a missing worker (`Ok(None)`)
    /// is `status = None`. Returns whether anything changed.
    pub(crate) fn apply(
        &mut self,
        status: Result<Option<StatusModel>, String>,
        records: Result<Vec<RecordModel>, String>,
    ) -> bool {
        let mut error = None;
        let mut next_status = self.status.clone();
        match status {
            Ok(status) => next_status = status,
            Err(message) => error = Some(message),
        }
        let mut next_records = self.records.clone();
        match records {
            Ok(records) => next_records = records,
            Err(message) => error = error.or(Some(message)),
        }
        let changed =
            next_status != self.status || next_records != self.records || error != self.error;
        self.status = next_status;
        self.records = next_records;
        self.error = error;
        changed
    }

    /// The rows of the state kept.
    pub(crate) fn rows(&self) -> Vec<VaultRow> {
        rows(self.status.as_ref(), &self.records)
    }

    /// The sites whose access ended since the last call: by a lock event of
    /// the worker newer than `seen_seq`, or by a state that was `Unlocked`
    /// and is not any more (also when the worker is gone or the entry was
    /// removed). An event number below the one seen means the worker
    /// restarted: its events count again from the start.
    pub(crate) fn newly_locked(&mut self, rows: &[VaultRow]) -> Vec<String> {
        let mut locked: Vec<String> = Vec::new();
        fn add(site: &str, locked: &mut Vec<String>) {
            if !locked.iter().any(|known| known == site) {
                locked.push(site.to_string());
            }
        }
        match &self.status {
            Some(status) => {
                if status.event_seq < self.seen_seq {
                    self.seen_seq = 0;
                }
                for event in &status.events {
                    if event.seq > self.seen_seq && event.is_lock() {
                        add(&event.site, &mut locked);
                    }
                }
                let newest = status
                    .events
                    .iter()
                    .map(|event| event.seq)
                    .max()
                    .unwrap_or(0);
                self.seen_seq = self.seen_seq.max(status.event_seq).max(newest);
            }
            None => self.seen_seq = 0,
        }
        for (site, was) in &self.previous {
            if *was != RowState::Unlocked {
                continue;
            }
            let now = rows
                .iter()
                .find(|row| &row.site == site)
                .map(|row| row.state);
            if now != Some(RowState::Unlocked) {
                add(site, &mut locked);
            }
        }
        self.previous = rows
            .iter()
            .map(|row| (row.site.clone(), row.state))
            .collect();
        locked
    }
}

/// One tab as far as a lock concerns it: the endpoint prefix of its remote
/// (`vault://<site>`), `None` for a tab without a vault.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TabModel {
    pub(crate) prefix: Option<String>,
}

/// What a lock does to a tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TabEffect {
    /// The tab returns to an empty state and shows this notice.
    ClearWithNotice { tab: usize, text: String },
}

/// Whether the prefix of a tab names the site (by its ID or by its label).
fn prefix_names(prefix: &str, site: &str, label: &str) -> bool {
    let Some(rest) = prefix.strip_prefix("vault://") else {
        return false;
    };
    let rest = rest.trim_end_matches('/');
    rest.eq_ignore_ascii_case(site) || (!label.is_empty() && rest == label)
}

/// A lock event of `site` (U29-2 a): every tab of the site is cleared. The
/// model of the tab loses its prefix, so the same event clears nothing twice.
pub(crate) fn apply_lock_event(tabs: &mut [TabModel], site: &str, label: &str) -> Vec<TabEffect> {
    let mut effects = Vec::new();
    for (index, tab) in tabs.iter_mut().enumerate() {
        let names = tab
            .prefix
            .as_deref()
            .is_some_and(|prefix| prefix_names(prefix, site, label));
        if names {
            tab.prefix = None;
            effects.push(TabEffect::ClearWithNotice {
                tab: index,
                text: format!("Tresor „{label}“ wurde gesperrt."),
            });
        }
    }
    effects
}

/// A failed request to the worker: the structured code and the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    pub(crate) code: Option<String>,
    pub(crate) message: String,
}

/// The text a failure shows: the text of its code (one wording for all
/// surfaces, `vault::error_text_for_code`), else the message of the worker.
pub(crate) fn failure_text(failure: &Failure) -> String {
    match failure.code.as_deref() {
        // The text of the code names a command line; the window says it
        // without one.
        Some("vault_locked") => "Der Tresor ist gesperrt.".to_string(),
        Some(code) => crate::vault::error_text_for_code(code)
            .map_or_else(|| failure.message.clone(), str::to_string),
        None => failure.message.clone(),
    }
}

/// What a successful unlock reports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct UnlockDone {
    pub(crate) remember_error: Option<String>,
}

/// What the unlock dialog does with an answer (U29-5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UnlockView {
    /// The red line under the field.
    pub(crate) error_line: Option<String>,
    /// The window stays open (after an error).
    pub(crate) keep_open: bool,
    /// The password field is emptied (after every answer).
    pub(crate) clear_password: bool,
    /// A notice for the main window.
    pub(crate) notice: Option<String>,
}

pub(crate) fn unlock_outcome(result: &Result<UnlockDone, Failure>) -> UnlockView {
    match result {
        Ok(done) => UnlockView {
            error_line: None,
            keep_open: false,
            clear_password: true,
            notice: done
                .remember_error
                .as_ref()
                .map(|text| format!("Passwort nicht gemerkt: {text}")),
        },
        Err(failure) => UnlockView {
            error_line: Some(failure_text(failure)),
            keep_open: true,
            clear_password: true,
            notice: None,
        },
    }
}

/// What the answer of `lock_vaults` leads to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LockStep {
    /// These sites are locked.
    Done(Vec<String>),
    /// Unsaved changes: ask „Sperren, Änderungen verschlüsselt aufbewahren“.
    AskKeep,
    Failed(String),
}

pub(crate) fn lock_step(result: &Result<Vec<String>, Failure>) -> LockStep {
    match result {
        Ok(sites) => LockStep::Done(sites.clone()),
        Err(failure) if failure.code.as_deref() == Some("vault_unsaved_changes") => {
            LockStep::AskKeep
        }
        Err(failure) => LockStep::Failed(failure_text(failure)),
    }
}

/// The arguments of `daemon::lock_vaults`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LockCall {
    pub(crate) site: Option<String>,
    pub(crate) keep_unsaved: bool,
}

/// The first attempt locks without keeping; the dialog answer „Sperren,
/// Änderungen verschlüsselt aufbewahren“ locks again with `keep_unsaved`.
pub(crate) fn lock_call(site: Option<&str>, keep_unsaved: bool) -> LockCall {
    LockCall {
        site: site.map(str::to_string),
        keep_unsaved,
    }
}

/// How a vault file opens (C7.2, U29-10): decided from the answer of
/// `mount_path_for` only; the mounting itself is never part of the decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OpenPlan {
    /// Open this path of the vault's drive.
    Immediate(PathBuf),
    /// Mount in the background, then open.
    Mount,
    /// No drive: ask for a cleartext copy, with the reason.
    AskCopy(String),
}

pub(crate) fn open_plan(
    location: &str,
    mount_on_open: bool,
    mount_path_for: &dyn Fn(&str) -> Result<Option<PathBuf>, String>,
) -> OpenPlan {
    match mount_path_for(location) {
        Ok(Some(path)) => OpenPlan::Immediate(path),
        Ok(None) if mount_on_open => OpenPlan::Mount,
        Ok(None) => OpenPlan::AskCopy("Der Tresor ist nicht als Laufwerk eingehängt".into()),
        Err(reason) => OpenPlan::AskCopy(reason),
    }
}

/// The page „Wiederherstellung“ of the properties (T12, T13): text only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecoveryPage {
    pub(crate) paragraphs: Vec<String>,
    /// Input fields on the page: none (C1.10, the word list is not free).
    pub(crate) inputs: usize,
    /// Buttons on the page: none.
    pub(crate) buttons: usize,
}

pub(crate) fn recovery_page() -> RecoveryPage {
    RecoveryPage {
        paragraphs: vec![
            crate::vault::recovery_key_info().note.to_string(),
            "Zum Zurücksetzen mit einem Wiederherstellungsschlüssel öffnen Sie den Tresor in \
             Cryptomator Desktop."
                .to_string(),
        ],
        inputs: 0,
        buttons: 0,
    }
}
