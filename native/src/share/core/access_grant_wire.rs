//! Stored form of an access grant (D.2.1) and the porting of grants stored
//! before it (C1).
//!
//! An Exec grant keeps the fields readers up to 0.5.182 know (`enabled`,
//! `run_as`, `programs`, `restricted_enabled`). A grant that those fields
//! express exactly is written in them byte for byte as before. Every other
//! grant is written with `enabled: false`, without `restricted_enabled`, with
//! the run-as account of its profile and with the object `access`, so older
//! readers see it switched off (fail closed). This version reads `access`
//! version 1; capability keys and fields it does not know are kept and never
//! take effect. An object of another version, or one it cannot read, turns
//! every capability off and is written back unchanged until the host changes
//! the grant.
use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::share::access_grant::{AccessGrant, AccountProfile, Capability, CapabilityDecision};
use crate::share::exec_policy::ExecRunAs;

/// Version of the `access` object this host reads and writes.
pub const ACCESS_VERSION: u64 = 1;

/// What a grant keeps of its stored `access` object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessRecord {
    /// Version 1: the grant, and the parts of the object it does not know.
    Known { grant: AccessGrant, kept: KeptParts },
    /// Another version, or an object this version cannot read.
    Unknown(Value),
}

/// Capability keys and fields of a version-1 object this version does not
/// know. They are written back and never take effect.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeptParts {
    capabilities: BTreeMap<String, Value>,
    fields: BTreeMap<String, Value>,
}

impl KeptParts {
    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty() && self.fields.is_empty()
    }
}

#[derive(Deserialize)]
struct AccessWire {
    version: u64,
    account: AccountProfile,
    #[serde(default)]
    capabilities: BTreeMap<String, Value>,
    #[serde(default)]
    valid_until: Option<i64>,
    #[serde(flatten)]
    fields: BTreeMap<String, Value>,
}

/// Reads a stored `access` object; never fails: what it cannot read is kept
/// as an unknown object with every capability off.
pub fn parse(value: Value) -> AccessRecord {
    match read_known(&value) {
        Some(record) => record,
        None => AccessRecord::Unknown(value),
    }
}

fn read_known(value: &Value) -> Option<AccessRecord> {
    if value.get("version").and_then(Value::as_u64) != Some(ACCESS_VERSION) {
        return None;
    }
    let wire: AccessWire = serde_json::from_value(value.clone()).ok()?;
    if wire.version != ACCESS_VERSION || wire.account.validate().is_err() {
        return None;
    }
    let mut grant = AccessGrant::new(wire.account);
    grant.valid_until = wire.valid_until;
    let mut kept = KeptParts {
        capabilities: BTreeMap::new(),
        fields: wire.fields,
    };
    for (key, decision) in wire.capabilities {
        match Capability::from_key(&key) {
            Some(capability) => {
                let decision = decision.as_str().and_then(CapabilityDecision::from_key)?;
                grant.set(capability, decision);
            }
            None => {
                kept.capabilities.insert(key, decision);
            }
        }
    }
    Some(AccessRecord::Known { grant, kept })
}

/// The `access` object of a version-1 grant. Capabilities that are off are
/// left out (missing means off).
pub fn known_value(grant: &AccessGrant, kept: &KeptParts) -> Value {
    let mut capabilities: Map<String, Value> = kept
        .capabilities
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    for (capability, decision) in grant.capabilities() {
        if decision.is_on() {
            capabilities.insert(capability.key().into(), Value::from(decision.key()));
        }
    }
    let mut object = Map::new();
    object.insert("version".into(), Value::from(ACCESS_VERSION));
    object.insert("account".into(), account_value(&grant.account));
    object.insert("capabilities".into(), Value::Object(capabilities));
    if let Some(end) = grant.valid_until {
        object.insert("valid_until".into(), Value::from(end));
    }
    for (key, value) in &kept.fields {
        object.entry(key.clone()).or_insert_with(|| value.clone());
    }
    Value::Object(object)
}

fn account_value(account: &AccountProfile) -> Value {
    match account {
        AccountProfile::Own => json!({ "kind": "own" }),
        AccountProfile::Remote => json!({ "kind": "remote" }),
        AccountProfile::Account { name } => json!({ "kind": "account", "name": name }),
    }
}

/// C1: what a grant stored without `access` means. A switched-on grant runs
/// commands and, without a program list, the terminal; a switched-off one
/// keeps its account and list but allows nothing.
pub fn ported(enabled: bool, run_as: &ExecRunAs, programs: &[String]) -> AccessGrant {
    let mut grant = AccessGrant::new(AccountProfile::from_run_as(run_as));
    if enabled {
        let terminal = if programs.is_empty() {
            CapabilityDecision::Allow
        } else {
            CapabilityDecision::Off
        };
        grant.set(Capability::ExecCommands, CapabilityDecision::Allow);
        grant.set(Capability::ExecTerminal, terminal);
    }
    grant
}

/// `enabled` and `run_as` of the fields readers up to 0.5.182 know, when they
/// express `grant` with `programs` exactly: own or named account, no end, no
/// `ask`, no screen capability, and commands and terminal `allow, allow`
/// without a list, `allow, off` with one, or `off, off`. `None`: the grant
/// needs the `access` object.
pub fn legacy_form(grant: &AccessGrant, programs: &[String]) -> Option<(bool, ExecRunAs)> {
    if grant.valid_until.is_some() || grant.account == AccountProfile::Remote {
        return None;
    }
    if grant.decision(Capability::ScreenView).is_on()
        || grant.decision(Capability::ScreenControl).is_on()
    {
        return None;
    }
    let commands = grant.decision(Capability::ExecCommands);
    let terminal = grant.decision(Capability::ExecTerminal);
    let enabled = match (commands, terminal) {
        (CapabilityDecision::Allow, CapabilityDecision::Allow) if programs.is_empty() => true,
        (CapabilityDecision::Allow, CapabilityDecision::Off) if !programs.is_empty() => true,
        (CapabilityDecision::Off, CapabilityDecision::Off) => false,
        _ => return None,
    };
    Some((enabled, grant.account.run_as()))
}

#[cfg(test)]
#[path = "access_grant_wire_tests.rs"]
mod tests;
