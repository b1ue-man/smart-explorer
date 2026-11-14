//! When the worker sets the login service up, syncs its grants or removes
//! it (E21). Pure decisions over facts the platform adapters collect; one
//! administrator approval per version, as the RDP self-setup.
use super::{machine::MachineIdentity, model::Policy};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginTrigger {
    /// A device received „Bildschirm steuern: Erlauben“ (RDP T1).
    Grant,
    /// First start of this version (RDP T2).
    VersionStart,
    /// A steering device became present (RDP T3).
    PeerOnline,
    /// A viewer asked for this host (RDP T4).
    PeerRequest,
    /// A device's steering ended (revocation, narrowing, expiry).
    CoverageEnded,
    /// The steering devices or their ends changed.
    ListChanged,
    /// The configured server's relays or the broker process changed, or the
    /// last sync did not reach the broker (`self_watch.rs`). Never prompts.
    Watch,
}

/// The service as the public broker evidence shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Installed {
    Missing,
    Running { release: String },
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelfState {
    Done,
    NotNeeded,
    Declined,
    Removed,
    RemoveDeclined,
    Failed(String),
}

impl SelfState {
    /// The German status text every surface shows (step 2 (g)).
    pub fn text(&self) -> String {
        match self {
            Self::Done => "eingerichtet; „Bildschirm steuern“ reicht auch für den Anmeldebildschirm".into(),
            Self::NotNeeded => "nicht nötig, kein Gerät darf diesen Bildschirm steuern".into(),
            Self::Declined => "Administratorfreigabe abgelehnt; die nächste Freigabe von „Bildschirm steuern“ fragt erneut".into(),
            Self::Removed => "entfernt, weil kein Gerät mehr steuern darf; die Maschinenkennung bleibt erhalten".into(),
            Self::RemoveDeclined => "Entfernen abgelehnt; der Dienst lässt kein Gerät mehr zu".into(),
            Self::Failed(reason) => format!("fehlgeschlagen: {}", reason_text(reason)),
        }
    }
}

/// German text of a failure reason; an unknown reason is shown as it is.
pub fn reason_text(reason: &str) -> String {
    match reason {
        "hash_mismatch" => "die Programmdatei weicht vom geprüften Stand ab; der bisherige Stand bleibt".into(),
        "image_missing" => "die Programmdatei für den Dienst fehlt neben se".into(),
        "grants_remain" => "andere Freigaben bestehen noch; der Dienst bleibt".into(),
        "not_privileged" => "der Einrichtungsschritt hat keine Administratorrechte".into(),
        "script_failed" => "der Einrichtungsschritt des Betriebssystems ist fehlgeschlagen; der bisherige Stand bleibt".into(),
        "stage_failed" => "der geschützte Arbeitsordner ist nicht anlegbar".into(),
        "invalid_step" => "ungültiger Einrichtungsschritt".into(),
        "unsupported" => "auf diesem System nicht verfügbar".into(),
        "no_interactive_session" => "keine angemeldete Sitzung für die Administratorfreigabe".into(),
        "no_prompt" => "keine Freigabeabfrage des Betriebssystems verfügbar (polkit oder Benutzerkontensteuerung)".into(),
        "se_missing" => "se fehlt neben dem Programm".into(),
        "service_unreachable" => "der Dienst ist nach der Einrichtung nicht erreichbar".into(),
        "grant_refused" => "der Dienst hat die Freigaben abgelehnt".into(),
        other => other.into(),
    }
}

/// Exit codes of `se rdp host-setup --system login-service…`.
pub mod exit {
    pub const DONE: i32 = 0;
    pub const HASH_MISMATCH: i32 = 40;
    pub const IMAGE_MISSING: i32 = 41;
    pub const GRANTS_REMAIN: i32 = 42;
    pub const NOT_PRIVILEGED: i32 = 43;
    pub const SCRIPT_FAILED: i32 = 44;
    pub const STAGE_FAILED: i32 = 45;
    pub const INVALID_STEP: i32 = 46;
    pub const UNSUPPORTED: i32 = 47;
    /// Windows `ERROR_CANCELLED` (UAC declined).
    pub const UAC_DECLINED: i32 = 1223;
    /// pkexec: the person dismissed the dialog or is not authorized.
    pub const PKEXEC_DECLINED: i32 = 126;
    /// pkexec: no authentication agent or authentication failed.
    pub const PKEXEC_UNAVAILABLE: i32 = 127;
}

/// The state an elevated step's exit code stands for; `removing` selects
/// the uninstall meanings of success and decline.
pub fn state_for_exit(code: i32, removing: bool) -> SelfState {
    let failed = |reason: &str| SelfState::Failed(reason.into());
    match code {
        exit::DONE if removing => SelfState::Removed,
        exit::DONE => SelfState::Done,
        exit::UAC_DECLINED | exit::PKEXEC_DECLINED if removing => SelfState::RemoveDeclined,
        exit::UAC_DECLINED | exit::PKEXEC_DECLINED => SelfState::Declined,
        exit::PKEXEC_UNAVAILABLE => failed("no_prompt"),
        exit::HASH_MISMATCH => failed("hash_mismatch"),
        exit::IMAGE_MISSING => failed("image_missing"),
        exit::GRANTS_REMAIN => failed("grants_remain"),
        exit::NOT_PRIVILEGED => failed("not_privileged"),
        exit::SCRIPT_FAILED => failed("script_failed"),
        exit::STAGE_FAILED => failed("stage_failed"),
        exit::INVALID_STEP => failed("invalid_step"),
        exit::UNSUPPORTED => failed("unsupported"),
        other => SelfState::Failed(format!("Code {other}")),
    }
}

/// The result of this version: `<sync_data_dir>/login-setup-<version>.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub state: SelfState,
    pub trigger: LoginTrigger,
    pub at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// No service and no steering device.
    Nothing,
    /// No new prompt this version; the marker's result stays.
    Keep(SelfState),
    /// Install or update with one approval, then sync.
    Install,
    /// Sync the grants and the network of the running service.
    Sync,
    /// No new prompt this version, but an older service still runs: it is
    /// synced all the same (a revocation must reach it); the status keeps
    /// the marker's result of this version, when there is one.
    SyncKeep(Option<SelfState>),
}

/// Only a grant asks again after a result of this version; without a
/// result every trigger but `Watch` may set up (that first run writes the
/// marker, so a version change leads to exactly one setup).
fn may_install(trigger: LoginTrigger, marker: Option<&Marker>) -> bool {
    match (trigger, marker) {
        (LoginTrigger::Watch, _) => false,
        (_, None) => true,
        (LoginTrigger::Grant, Some(_)) => true,
        (LoginTrigger::PeerRequest, Some(marker)) => marker.state != SelfState::Declined,
        _ => false,
    }
}

pub fn decide(
    trigger: LoginTrigger,
    steering: bool,
    installed: &Installed,
    marker: Option<&Marker>,
    version: &str,
) -> Plan {
    match installed {
        Installed::Unavailable if steering => Plan::Keep(SelfState::Failed("unsupported".into())),
        Installed::Unavailable | Installed::Missing if !steering => Plan::Nothing,
        Installed::Running { release } if release == version || !steering => Plan::Sync,
        _ if may_install(trigger, marker) => Plan::Install,
        Installed::Running { .. } => Plan::SyncKeep(marker.map(|marker| marker.state.clone())),
        _ => Plan::Keep(
            marker
                .map(|marker| marker.state.clone())
                .unwrap_or(SelfState::Declined),
        ),
    }
}

/// After a sync: the last steering device is gone and no grant remains,
/// so the service is removed — once per version (step 2 (f)).
pub fn removal_due(
    steering: bool,
    own_grants: usize,
    other_grants: usize,
    marker: Option<&Marker>,
) -> bool {
    !steering
        && own_grants == 0
        && other_grants == 0
        && !marker.is_some_and(|marker| {
            matches!(marker.state, SelfState::Removed | SelfState::RemoveDeclined)
        })
}

/// Step 2 (c): the source and the protected copy must both carry exactly
/// the hash the worker bound the approval to; any mismatch stops before a
/// file of the installation changes.
pub fn image_binding(expected: &str, source: &str, staged: &str) -> Result<(), &'static str> {
    let valid = expected.len() == 64 && expected.bytes().all(|byte| byte.is_ascii_hexdigit());
    if !valid || !expected.eq_ignore_ascii_case(source) || !expected.eq_ignore_ascii_case(staged) {
        return Err("hash_mismatch");
    }
    Ok(())
}

/// The policy an elevated setup installs: the existing one (grants kept,
/// next revision) when it belongs to `identity`; otherwise one for
/// `identity` that keeps the existing grants, revision after the old one.
pub fn install_policy(existing: Option<&Policy>, identity: &MachineIdentity) -> Policy {
    let revision = existing
        .map(|policy| policy.revision.saturating_add(1))
        .unwrap_or(1);
    Policy {
        format: super::model::VERSION,
        revision,
        machine_id: identity.machine_id.clone(),
        machine_public_key: identity.public_key.clone(),
        // Both installers replace this with the gateway UID or service SID.
        gateway_identity: "SET_BY_ADMINISTRATIVE_INSTALLER".into(),
        grants: existing
            .map(|policy| policy.grants.clone())
            .unwrap_or_default(),
    }
}

/// The elevated step names: `login-service:<sha256>` and `login-service-remove`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Install { sha256: String },
    Remove,
}

pub const STEP_INSTALL: &str = "login-service:";
pub const STEP_REMOVE: &str = "login-service-remove";

impl Step {
    pub fn parse(step: &str) -> Option<Self> {
        if step == STEP_REMOVE {
            return Some(Self::Remove);
        }
        let hash = step.strip_prefix(STEP_INSTALL)?;
        (hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())).then(|| {
            Self::Install {
                sha256: hash.to_ascii_lowercase(),
            }
        })
    }
    pub fn name(&self) -> String {
        match self {
            Self::Install { sha256 } => format!("{STEP_INSTALL}{sha256}"),
            Self::Remove => STEP_REMOVE.into(),
        }
    }
}

#[cfg(test)]
#[path = "self_plan_task_tests.rs"]
mod self_plan_task_tests;
