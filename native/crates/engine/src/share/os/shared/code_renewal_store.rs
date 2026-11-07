//! Durable half of the Direct-code renewal (E17): the renewal file
//! `share_code_renewal.json`, the old secrets in the secure store, the
//! trigger after a removal, confirmations and the process view.
//!
//! Every function here runs under the identity transaction guard (process
//! mutex and system lock), so identity file, renewal file and secure store
//! change together. The state machine is `core/code_renewal_state.rs`.
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use zeroize::Zeroizing;

use crate::share::identity::code_renewal::{
    CodeRenewalState, RenewalNeed, RetiredLookups, MAX_RETIRING_CODES,
};
use crate::share::identity::code_renewal_registry::{self as registry, RetiringMaterial};
use crate::share::identity::{IdentityPersistence, ShareIdentity};
use crate::share::profiles::ShareProfiles;

const RENEWAL_FILE: &str = "share_code_renewal.json";
const MAX_RENEWAL_BYTES: u64 = 64 * 1024;

/// On every production identity load: completes a prepared renewal,
/// renews after a removal the last renewal has not answered, finishes
/// generations nobody waits for and installs the process view. Never fails
/// the load: a failure keeps the current data and shows in the status.
pub(super) fn settle_on_load(
    default_home: Option<String>,
    identity: ShareIdentity,
    storage: &mut impl IdentityPersistence,
) -> ShareIdentity {
    let mut state = match load_state() {
        Ok(state) => state,
        Err(error) => {
            registry::record_error(&identity.node_id, error);
            return identity;
        }
    };
    let before = state.clone();
    // Unreadable profiles postpone pruning and renewal; the profile error
    // itself is reported by the profile surfaces, not as a renewal failure.
    let profiles = ShareProfiles::load_checked(default_home.clone());
    let mut error = None;
    let mut current = identity;
    let mut retired = state.settle(&current.direct_lookup_id);
    let mut need = RenewalNeed::Nothing;
    if let Ok(profiles) = &profiles {
        retired.extend(state.prune(|peer| profiles.holds_direct_code(peer)));
        need = state.need(profiles.latest_direct_removal());
    }
    let mut saved = false;
    match (need, &profiles) {
        (RenewalNeed::Initialize(latest), _) => state.initialize(latest),
        (RenewalNeed::Renew(latest), Ok(profiles)) => {
            match renew(&mut state, &current, profiles, Some(latest), storage) {
                Ok((renewed, more)) => {
                    current = renewed;
                    retired.extend(more);
                    saved = true;
                }
                Err(renew_error) => error = Some(renew_error),
            }
        }
        _ => {}
    }
    if !saved && state != before {
        if let Err(save_error) = save_state(&state) {
            // Nothing is deleted while the file still names it.
            retired.clear();
            error = Some(save_error);
        }
    }
    if let Err(finish_error) = finish_retired(&default_home, storage, &retired) {
        error = Some(finish_error);
    }
    install(&current, &state, profiles.as_ref().ok(), storage, error);
    current
}

/// T1m: the manual renewal („Code neu generieren“) through the same
/// transition instead of suspending every grant.
pub(super) fn renew_manually(
    default_home: Option<String>,
    identity: ShareIdentity,
    storage: &mut impl IdentityPersistence,
) -> Result<ShareIdentity, String> {
    let mut state = load_state()?;
    let profiles = ShareProfiles::load_checked(default_home.clone())?;
    if state.retiring.len() >= MAX_RETIRING_CODES {
        return Err(format!(
            "Es laufen bereits {MAX_RETIRING_CODES} Code-Erneuerungen; erst wenn die Geräte einer \
             davon bestätigt haben, ist eine weitere möglich"
        ));
    }
    let (renewed, retired) = renew(&mut state, &identity, &profiles, None, storage)?;
    let finished = finish_retired(&default_home, storage, &retired).err();
    install(&renewed, &state, Some(&profiles), storage, finished);
    Ok(renewed)
}

/// T2 from a worker: the device `(device_id, public_key, peer_node)` now
/// holds `accepted_lookup_id`. Cheap when no renewal waits for it.
pub(in crate::share) fn confirm_peer(
    node_id: &str,
    device_id: &str,
    public_key: &str,
    peer_node: &str,
    accepted_lookup_id: &str,
) {
    if !registry::is_pending(node_id, device_id, public_key) {
        return;
    }
    update(node_id, |state, current| {
        state.confirm(
            device_id,
            public_key,
            peer_node,
            accepted_lookup_id,
            current,
        )
    });
}

/// T3 from a worker: the device used `lookup_id` without renewal support.
pub(in crate::share) fn mark_legacy(
    node_id: &str,
    device_id: &str,
    public_key: &str,
    peer_node: &str,
    lookup_id: &str,
) {
    if !registry::is_pending(node_id, device_id, public_key) {
        return;
    }
    update(node_id, |state, _| {
        state.mark_legacy(device_id, public_key, peer_node, lookup_id);
        Vec::new()
    });
}

fn update(node_id: &str, change: impl FnOnce(&mut CodeRenewalState, &str) -> RetiredLookups) {
    let result = (|| -> Result<(), String> {
        let _guard = super::identity_transaction_guard()
            .map_err(|error| format!("Share-Identitaet sperren: {error}"))?;
        let mut storage = super::SystemIdentityPersistence;
        let identity = ShareIdentity::load_or_create_with(String::new(), &mut storage)?;
        if identity.node_id != node_id {
            return Ok(());
        }
        let mut state = load_state()?;
        let before = state.clone();
        let retired = change(&mut state, &identity.direct_lookup_id);
        if state != before {
            save_state(&state)?;
        }
        let home = Some(super::default_home());
        let profiles = ShareProfiles::load_checked(home.clone()).ok();
        let finished = finish_retired(&home, &mut storage, &retired).err();
        install(&identity, &state, profiles.as_ref(), &mut storage, finished);
        Ok(())
    })();
    if let Err(error) = result {
        registry::record_error(node_id, error);
    }
}

/// Two-phase renewal (state machine T1): new secret → renewal file with
/// `prepared` → identity file → `prepared` settled. A failure before the
/// identity file leaves the current data current.
fn renew(
    state: &mut CodeRenewalState,
    identity: &ShareIdentity,
    profiles: &ShareProfiles,
    covered_removals_at: Option<i64>,
    storage: &mut impl IdentityPersistence,
) -> Result<(ShareIdentity, RetiredLookups), String> {
    let candidate = identity.allocate_renewed_with(storage)?;
    let before = state.clone();
    state.prepare(
        &identity.direct_lookup_id,
        &candidate.direct_lookup_id,
        profiles.direct_code_holders(),
        covered_removals_at,
        now_secs(),
    );
    if let Err(error) = save_state(state) {
        *state = before;
        let _ = ShareIdentity::delete_retiring_secret_with(storage, &candidate.direct_lookup_id);
        return Err(error);
    }
    if let Err(error) = candidate.commit_renewed_with(storage) {
        let orphans = state.settle(&identity.direct_lookup_id);
        // Unsaved, the prepared file rolls back on the next load instead.
        if save_state(state).is_ok() {
            for lookup_id in orphans {
                let _ = ShareIdentity::delete_retiring_secret_with(storage, &lookup_id);
            }
        }
        return Err(error);
    }
    let retired = state.settle(&candidate.direct_lookup_id);
    // Unsaved, the next load settles the same way; until then nothing that
    // the file still names is deleted.
    let retired = match save_state(state) {
        Ok(()) => retired,
        Err(_) => Vec::new(),
    };
    Ok((candidate, retired))
}

/// Z2: old secrets and request records of finished generations.
fn finish_retired(
    default_home: &Option<String>,
    storage: &mut impl IdentityPersistence,
    retired: &[String],
) -> Result<(), String> {
    let mut failures = Vec::new();
    for lookup_id in retired {
        if let Err(error) = ShareIdentity::delete_retiring_secret_with(storage, lookup_id) {
            failures.push(error);
        }
        let names = ShareProfiles::load_checked(default_home.clone())
            .map(|profiles| profiles.names_direct_lookup(lookup_id))
            .unwrap_or(false);
        if names {
            if let Err(error) = ShareProfiles::mutate_persisted(default_home.clone(), |profiles| {
                profiles.forget_retired_direct_lookup(lookup_id);
                Ok(())
            }) {
                failures.push(error);
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn install(
    identity: &ShareIdentity,
    state: &CodeRenewalState,
    profiles: Option<&ShareProfiles>,
    storage: &mut impl IdentityPersistence,
    mut error: Option<String>,
) {
    let mut materials = Vec::new();
    for code in &state.retiring {
        match ShareIdentity::retiring_secret_with(storage, &code.lookup_id) {
            Ok(Some(mut secret)) => {
                materials.push(RetiringMaterial {
                    lookup_id: code.lookup_id.clone(),
                    secret: Zeroizing::new(secret.to_vec()),
                    pending: code
                        .pending
                        .iter()
                        .map(|peer| {
                            (
                                peer.device_id.clone(),
                                peer.public_key.clone(),
                                peer.node_id.clone(),
                            )
                        })
                        .collect(),
                });
                secret.fill(0);
            }
            Ok(None) => {
                error = Some(format!(
                    "Auslaufender Direkt-Code {} fehlt im sicheren Speicher",
                    code.lookup_id
                ))
            }
            Err(load_error) => error = Some(load_error),
        }
    }
    let deferred = profiles.is_some_and(|profiles| {
        state.need(profiles.latest_direct_removal()) == RenewalNeed::Deferred
    });
    let withdrawn = profiles
        .map(ShareProfiles::withdrawn_direct_keys)
        .unwrap_or_default();
    registry::install(
        &identity.node_id,
        &identity.direct_lookup_id,
        materials,
        withdrawn,
        state.status(deferred, error),
    );
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or_default()
}

fn renewal_path() -> PathBuf {
    crate::support_dirs::app_data_file(RENEWAL_FILE)
}

fn load_state() -> Result<CodeRenewalState, String> {
    let path = renewal_path();
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(CodeRenewalState::default())
        }
        Err(error) => return Err(format!("Code-Erneuerung lesen: {error}")),
    };
    if !metadata.file_type().is_file() || metadata.len() > MAX_RENEWAL_BYTES {
        return Err("Code-Erneuerung ist keine reguläre Datei bis 64 KiB".into());
    }
    let mut bytes = Vec::new();
    crate::support_dirs::open_private_file(&path)
        .map_err(|error| format!("Code-Erneuerung lesen: {error}"))?
        .take(MAX_RENEWAL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Code-Erneuerung lesen: {error}"))?;
    if bytes.len() as u64 > MAX_RENEWAL_BYTES {
        return Err("Code-Erneuerung überschreitet 64 KiB".into());
    }
    serde_json::from_slice(&bytes).map_err(|error| format!("Code-Erneuerung beschädigt: {error}"))
}

fn save_state(state: &CodeRenewalState) -> Result<(), String> {
    let contents = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("Code-Erneuerung kodieren: {error}"))?;
    if contents.len() as u64 > MAX_RENEWAL_BYTES {
        return Err("Code-Erneuerung überschreitet 64 KiB".into());
    }
    write_atomic(&renewal_path(), &contents)
        .map_err(|error| format!("Code-Erneuerung speichern: {error}"))
}

/// Same staging and replacement as `share_identity.json`.
fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    crate::support_dirs::ensure_private_dir(parent)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut staged = None;
    for attempt in 0..1000u32 {
        let candidate = path.with_extension(format!(
            "se-renewal-{}-{nonce:x}-{attempt:x}.tmp",
            std::process::id()
        ));
        match crate::support_dirs::create_private_file(&candidate) {
            Ok(mut file) => {
                let result = file
                    .write_all(contents)
                    .and_then(|()| file.flush())
                    .and_then(|()| file.sync_all());
                if let Err(error) = result {
                    drop(file);
                    let _ = std::fs::remove_file(&candidate);
                    return Err(error);
                }
                staged = Some(candidate);
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    let staged = staged.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate the renewal staging file",
        )
    })?;
    let unicode = |path: &Path| {
        path.to_str()
            .map(str::to_owned)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "path is not Unicode"))
    };
    let staged_text = unicode(&staged)?;
    let path_text = unicode(path)?;
    let backend = crate::vfs::LocalBackend::new("/");
    let result = crate::vfs::promote_staged_replace(&backend, &staged_text, &path_text);
    if result.is_err() {
        let _ = std::fs::remove_file(staged);
    }
    result
}
