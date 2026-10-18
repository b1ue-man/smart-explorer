//! Access rights of this phone as Exec host (PRM-M10, D.2.4): `share.setAccess`,
//! the open confirmations (`share.accessApprovals`), the host user's answer to
//! exactly one of them (`share.decideAccessApproval`), the report that the app
//! can show one nowhere (`share.reportApprovalUndeliverable`; the host then
//! refuses the start with `approval_unavailable`), the fields
//! `execProvider.accountChoices`, `otherAccountsNote`, `elevated`,
//! `capabilities` and `execTargets[].access` of `share.status`, and the app's
//! `ConsentNotifier`. Android runs commands only as the app's own account
//! (C4), so the bridge offers no other one. Screen capabilities get no rows
//! and no change before SCR-M8 gives Android a screen host (W1). The notifier
//! only wakes the Share poller: its `share` event makes the app read the open
//! requests and show them (a dialog while the app is visible, else a
//! notification of the channel `access_approval`).
//!
//! Submodule of `share_exec.rs`; the routes of `share.*` reach it as
//! `share_exec::access`.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Map, Value};

use super::super::args::{invalid, now_secs, str_arg};
use super::super::share_state::{committed, default_home, wake};
use super::{grant_answer, program_settings, provider};
use crate::mobile::ApiError;
use crate::share::access_approval::{ApprovalDecision, ApprovalId, ApprovalView, ConsentNotifier};
use crate::share::access_catalog::{
    capability_catalog, CapabilityEntry, HelperState, HelperStatus, HostFacts,
};
use crate::share::access_grant::{
    AccessGrant, AccessGrantChange, AccessGrantView, AccountProfile, Capability,
    CapabilityDecision, Expiry,
};
use crate::share::consent_launch::{capability_label, request_line};
use crate::share::exec_access_view::ApprovalSurface;
use crate::share::{resolve_exec_target, ExecProviderStatus, ShareProfiles};

/// „Ausführen als“ on Android, word for word as decided in C4.
pub(super) const OTHER_ACCOUNTS_NOTE: &str = concat!(
    "App-Konto von Smart Explorer. Befehle anderer Geräte können die Daten der App lesen ",
    "(gespeicherte Verbindungspasswörter, Share-Identität, Google-Drive-Tokens) und alle ",
    "Dateien, auf die die App zugreifen darf. Ein Konto außer dem App-Konto ist auf Android ",
    "nicht möglich: Eine App kann keinen Prozess unter einer fremden Nutzerkennung starten. ",
    "Root (über su) und das Shell-Konto (über Shizuku) laufen außerhalb der App, sodass ein ",
    "abgebrochener Befehl nicht sicher beendet werden kann. Ein zweites Android-Profil ",
    "(Arbeitsprofil oder privater Bereich) ist ein eigenes Gerät: dort Smart Explorer ",
    "installieren und koppeln."
);

/// Changes each time a request opens or ends on this phone; part of
/// `share_exec::host_activity`, so the poller sends `share` for it.
static ACTIVITY: AtomicU64 = AtomicU64::new(0);

/// Key of a capability in the bridge (`share.setAccess`, `access`,
/// `share.accessApprovals`), as `CapabilityEntry::bridge_key` names it.
pub(super) fn bridge_key(capability: Capability) -> &'static str {
    match capability {
        Capability::ExecCommands => "execCommands",
        Capability::ExecTerminal => "execTerminal",
        Capability::ScreenView => "screenView",
        Capability::ScreenControl => "screenControl",
    }
}

fn capability_of(key: &str) -> Option<Capability> {
    Capability::ALL
        .into_iter()
        .find(|capability| bridge_key(*capability) == key)
}

/// The rows this phone offers: the catalog of an Android host, without the
/// screen rows until SCR-M8 adds a screen host and its consent activity (W1).
pub(super) fn android_rows() -> Vec<CapabilityEntry> {
    let facts = HostFacts {
        graphical_session: crate::screen::graphical_session(),
        account_helper: HelperStatus {
            state: HelperState::Unavailable,
            version: None,
            detail: None,
        },
    };
    capability_catalog(&facts)
        .into_iter()
        .filter(|entry| entry.capability.is_exec())
        .collect()
}

/// The fields PRM-M10 adds to `execProvider` of `share.status`: the one
/// account choice of Android (the app's own, form of `se share exec accounts
/// --json` in camelCase), why there is no other one (C4), whether the app's
/// account is elevated, and the capability rows of the access dialog.
pub(super) fn provider_fields(status: &ExecProviderStatus) -> Map<String, Value> {
    let rows: Vec<Value> = android_rows()
        .iter()
        .map(|entry| {
            json!({
                "key": entry.bridge_key,
                "label": entry.label,
                "description": entry.description,
                "usesAccount": entry.uses_account,
            })
        })
        .collect();
    let mut fields = Map::new();
    fields.insert(
        "accountChoices".into(),
        json!([{
            "profile": "own",
            "label": status.user_label,
            "available": true,
            "elevated": status.elevated,
            "adminGroup": false,
        }]),
    );
    fields.insert("otherAccountsNote".into(), json!(OTHER_ACCOUNTS_NOTE));
    fields.insert("elevated".into(), json!(status.elevated));
    fields.insert("capabilities".into(), Value::Array(rows));
    fields
}

/// `access` of an `execTargets` entry; `own_label` names the app's account.
pub(super) fn access_json(grant: &AccessGrant, own_label: &str, now: i64) -> Value {
    view_json(&AccessGrantView::new(grant, own_label, now))
}

/// `{account, accountLabel, capabilities:{execCommands, execTerminal,
/// screenView?, screenControl?}, validUntil, expired}` (D.2.4).
pub(super) fn view_json(view: &AccessGrantView) -> Value {
    let capabilities: Map<String, Value> = view
        .capabilities
        .iter()
        .map(|(capability, decision)| (bridge_key(*capability).to_string(), json!(decision.key())))
        .collect();
    json!({
        "account": serde_json::to_value(&view.account).unwrap_or(Value::Null),
        "accountLabel": view.account_label,
        "capabilities": capabilities,
        "validUntil": view.valid_until,
        "expired": view.expired,
    })
}

/// `share.setAccess {targetKey, account?, capabilities?, programs?,
/// validUntil?}` → `{revision, access}`. A missing field keeps its value,
/// `validUntil: null` removes the end. The change is checked against the
/// stored grant before anything is sent; like `share.setExec`, a capability
/// turned on needs the exec provider and an active relation, and only a
/// change that is stored and applied counts.
pub(in super::super) fn set_access(args: &Value) -> Result<Value, ApiError> {
    let key = str_arg(args, "targetKey")?;
    let change = access_change(args, now_secs())?;
    let profiles = ShareProfiles::load_checked(default_home())
        .map_err(|error| ApiError::internal(format!("Share-Profile nicht lesbar: {error}")))?;
    let view = resolve_exec_target(&profiles, key).ok_or_else(|| {
        ApiError::not_found(
            "Gerät nicht gefunden oder seine Identität hat sich geändert – bitte neu laden.",
        )
    })?;
    let mut probe = view.access.clone();
    let mut programs = view.programs.clone();
    probe
        .apply(&change, &mut programs)
        .map_err(|error| invalid(format!("Zugriff ungültig: {error}")))?;
    if change
        .capabilities
        .values()
        .any(|decision| decision.is_on())
    {
        let provider = provider();
        if !provider.available {
            return Err(ApiError::unsupported(format!(
                "Befehle auf diesem Telefon sind nicht verfügbar: {}",
                provider.detail
            )));
        }
        if !view.base_authorized {
            return Err(ApiError::new(
                "conflict",
                "Die Freigabe für dieses Gerät ist nicht aktiv; Befehle lassen sich nicht erlauben.",
            ));
        }
    }
    let (result, grant) = crate::daemon::mutate_access_grant(view.target, change, None)
        .map_err(|error| ApiError::internal(format!("Zugriff nicht geändert: {error}")))?;
    let mut answer = grant_answer(
        result.persisted,
        result.applied,
        result.error.as_deref(),
        result.revision,
    )?;
    // `None`: the worker first finished an earlier change; this one waits.
    let grant = grant.ok_or_else(|| {
        ApiError::new(
            "conflict",
            "Eine frühere Änderung wurde erst abgeschlossen; bitte die Änderung wiederholen.",
        )
    })?;
    answer["access"] = view_json(&grant);
    if let Ok(stored) = ShareProfiles::load_checked(default_home()) {
        committed(stored);
    }
    wake();
    Ok(answer)
}

/// The arguments of `share.setAccess` as a change. `account`,
/// `capabilities` and `programs` that are missing or `null` keep their value;
/// `validUntil` missing keeps the end, `null` removes it, a number (Unix
/// seconds) after `now` sets it. A change without any field is refused.
pub(super) fn access_change(args: &Value, now: i64) -> Result<AccessGrantChange, ApiError> {
    let mut change = AccessGrantChange::default();
    if let Some(account) = present(args, "account") {
        change.account = Some(account_of(account)?);
    }
    if let Some(capabilities) = present(args, "capabilities") {
        change.capabilities = capabilities_of(capabilities)?;
    }
    if present(args, "programs").is_some() {
        change.programs = program_settings(args)?.map(|settings| settings.programs);
    }
    match args.get("validUntil") {
        None => {}
        Some(Value::Null) => change.expiry = Some(Expiry::None),
        Some(value) => {
            let end = value
                .as_i64()
                .ok_or_else(|| invalid("„validUntil“ ist keine Zahl (Sekunden seit 1970)."))?;
            if end <= now {
                return Err(invalid("Das Ende des Zugriffs muss in der Zukunft liegen."));
            }
            change.expiry = Some(Expiry::At(end));
        }
    }
    if change.is_empty() {
        return Err(invalid(
            "Keine Änderung angegeben (account, capabilities, programs oder validUntil).",
        ));
    }
    Ok(change)
}

fn present<'a>(args: &'a Value, key: &str) -> Option<&'a Value> {
    args.get(key).filter(|value| !value.is_null())
}

/// `{"kind":"own"}`; Android has no other account (C4).
fn account_of(value: &Value) -> Result<AccountProfile, ApiError> {
    let account: AccountProfile = serde_json::from_value(value.clone())
        .map_err(|_| invalid("Ungültiges Konto; erwartet wird {\"kind\":\"own\"}."))?;
    if account != AccountProfile::Own {
        return Err(ApiError::unsupported(
            "Auf Android laufen Befehle nur unter dem App-Konto von Smart Explorer.",
        ));
    }
    Ok(account)
}

/// Only the named capabilities change; each one this phone offers is
/// `off`, `ask` or `allow`.
fn capabilities_of(value: &Value) -> Result<BTreeMap<Capability, CapabilityDecision>, ApiError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("„capabilities“ ist kein Objekt."))?;
    let offered = android_rows();
    let mut decisions = BTreeMap::new();
    for (key, decision) in object {
        let capability =
            capability_of(key).ok_or_else(|| invalid(format!("Unbekannte Fähigkeit: {key}")))?;
        if !offered.iter().any(|row| row.capability == capability) {
            return Err(ApiError::unsupported(format!(
                "„{key}“ gibt es auf diesem Telefon nicht."
            )));
        }
        let decision = decision
            .as_str()
            .and_then(CapabilityDecision::from_key)
            .ok_or_else(|| {
                invalid(format!(
                    "Ungültige Entscheidung für {key}: erwartet off, ask oder allow."
                ))
            })?;
        decisions.insert(capability, decision);
    }
    Ok(decisions)
}

/// `share.accessApprovals {}` → `{pending:[…]}`, oldest first; empty while
/// no worker runs.
pub(in super::super) fn approvals() -> Result<Value, ApiError> {
    let pending = crate::daemon::access_approvals()
        .map_err(|error| ApiError::internal(format!("Rückfragen nicht lesbar: {error}")))?;
    Ok(approvals_json(&pending))
}

/// Each request as `{id, deviceName, deviceId, fingerprint, relation, room,
/// capability, capabilityLabel, text, accountLabel, command, requestedAt,
/// surface, olderClientTerminal}`; `capability` is the bridge key, `command`
/// `{mode:"argv",program,args}`, `{mode:"shell",text}`, `{mode:"login"}` or
/// `null` (screen). The texts come from the host, cut and cleaned.
pub(super) fn approvals_json(pending: &[ApprovalView]) -> Value {
    let pending: Vec<Value> = pending
        .iter()
        .map(|view| {
            json!({
                "id": view.id.as_str(),
                "deviceName": view.device_name,
                "deviceId": view.device_id,
                "fingerprint": view.fingerprint,
                "relation": view.relation,
                "room": view.room,
                "capability": bridge_key(view.capability),
                "capabilityLabel": capability_label(view.capability),
                "text": request_line(view),
                "accountLabel": view.account_label,
                "command": view
                    .command
                    .as_ref()
                    .map(|command| serde_json::to_value(command).unwrap_or(Value::Null)),
                "requestedAt": view.requested_at,
                "surface": view.surface,
                "olderClientTerminal": view.older_client_terminal,
            })
        })
        .collect();
    json!({ "pending": pending })
}

/// `share.decideAccessApproval {approvalId, decision}` → `{}`: the answer to
/// exactly this request (`once`, `always`, `deny`). A request that is no
/// longer open is `not_found`.
pub(in super::super) fn decide(args: &Value) -> Result<Value, ApiError> {
    let (id, decision) = decision_args(args)?;
    crate::daemon::decide_access_approval(id.as_str(), decision).map_err(answer_error)?;
    wake();
    Ok(json!({}))
}

/// `share.reportApprovalUndeliverable {approvalId}` → `{}`: the app can show
/// this request neither as a dialog nor as a notification; the host refuses
/// the start with `approval_unavailable` (never allows it).
pub(in super::super) fn report_undeliverable(args: &Value) -> Result<Value, ApiError> {
    let id = approval_id(args)?;
    crate::daemon::report_approval_undeliverable(id.as_str()).map_err(answer_error)?;
    wake();
    Ok(json!({}))
}

/// The request and the answer named by the arguments; nothing else is read.
pub(super) fn decision_args(args: &Value) -> Result<(ApprovalId, ApprovalDecision), ApiError> {
    let id = approval_id(args)?;
    let decision = match str_arg(args, "decision")? {
        "once" => ApprovalDecision::Once,
        "always" => ApprovalDecision::Always,
        "deny" => ApprovalDecision::Deny,
        other => {
            return Err(invalid(format!(
                "Unbekannte Entscheidung „{other}“: erwartet once, always oder deny."
            )))
        }
    };
    Ok((id, decision))
}

fn approval_id(args: &Value) -> Result<ApprovalId, ApiError> {
    ApprovalId::parse(str_arg(args, "approvalId")?)
        .map_err(|_| invalid("Ungültige Rückfrage-ID (32 Hexadezimalzeichen)."))
}

fn answer_error(error: String) -> ApiError {
    if error.contains("no longer open") {
        ApiError::not_found("Die Rückfrage ist nicht mehr offen.")
    } else {
        ApiError::internal(format!("Antwort nicht übernommen: {error}"))
    }
}

/// How this phone asks its user (C5): the app shows the request, as a dialog
/// while it is visible, else as a notification; when it can do neither it
/// reports `share.reportApprovalUndeliverable`. Showing only wakes the Share
/// poller, so the host never waits on the app here.
pub(super) struct AppConsent;

impl ConsentNotifier for AppConsent {
    fn show(&self, _request: &ApprovalView) -> Option<ApprovalSurface> {
        changed();
        Some(ApprovalSurface::Notification)
    }

    fn withdraw(&self, _id: &ApprovalId) {
        changed();
    }
}

fn changed() {
    ACTIVITY.fetch_add(1, Ordering::AcqRel);
    wake();
}

/// Requests opened or ended so far (see `ACTIVITY`).
pub(super) fn activity() -> u64 {
    ACTIVITY.load(Ordering::Acquire)
}

/// From now on this process asks through the app (before: nobody, so every
/// request ended as `approval_unavailable`).
#[cfg(target_os = "android")]
pub(super) fn install_notifier() {
    crate::share::consent_launch::approvals().set_notifier(std::sync::Arc::new(AppConsent));
}

#[cfg(test)]
#[path = "share_access_tests.rs"]
mod tests;
