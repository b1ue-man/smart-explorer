//! „Zusammenführen“ of a text conflict (vault plan C6.4, U26-6): the two
//! versions are decrypted into memory only, the surface shows them and
//! sends the merged text back; the engine gets the **cipher bytes** of both
//! originals (their signatures are the signatures of the conflict) and the
//! merged text encrypted anew as `MergeChoice::Write`, so `*.merge-*.inputs`
//! holds ciphertext only and never a cleartext line. Without a version that
//! is valid UTF-8 and at most `MERGE_MAX_BYTES` the merge is not offered.
use std::io::{self, Read};
use std::sync::atomic::AtomicBool;

use zeroize::Zeroizing;

use crate::bisync::{merge_recorded_for_key, Conflict, MergeChoice, OriginalContent};
use crate::cryptomator::{cleartext_file_size, decrypt_bytes, encrypt_bytes, file_size};
use crate::vault::core::compare_model::ItemAction;
use crate::vault::core::path_map::storage_path;
use crate::vault::core::VaultSession;

use super::compare::invalid;
use super::compare_cache::{cache, CompareState, Payload, SideIo};

/// The largest version the merge view takes: both versions travel in one
/// line of the service (2 MiB with the escaping of JSON) and the surface
/// shows the line view whole; larger files offer only „Fassung A/B
/// behalten“.
pub(crate) const MERGE_MAX_BYTES: u64 = 512 * 1024;

/// The stored bytes of the file `rel` on `side` (at most the cipher size of
/// `MERGE_MAX_BYTES`).
fn cipher_bytes(side: &SideIo, rel: &str, session: &VaultSession) -> io::Result<Vec<u8>> {
    let limit = file_size(MERGE_MAX_BYTES, session.config().combo);
    let reader = side.backend.open_read(&storage_path(&side.root, rel))?;
    let mut bytes = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(invalid("Die Datei ist für das Zusammenführen zu groß"));
    }
    Ok(bytes)
}

/// The cleartext of the file `rel` on `side`, in wiped memory.
pub(super) fn read_clear(
    side: &SideIo,
    rel: &str,
    session: &VaultSession,
) -> io::Result<Zeroizing<Vec<u8>>> {
    let bytes = cipher_bytes(side, rel, session)?;
    let limit = usize::try_from(MERGE_MAX_BYTES).unwrap_or(usize::MAX);
    decrypt_bytes(&session.cryptor(), &bytes, limit).map_err(|error| session.format_error(error))
}

/// Whether both versions of `conflict` can be merged as text.
pub(super) fn mergeable(sides: &[SideIo], session: &VaultSession, conflict: &Conflict) -> bool {
    let (Some(a), Some(b)) = (conflict.a, conflict.b) else {
        return false;
    };
    if conflict.duplicates.is_some() || sides.len() < 2 {
        return false;
    }
    let combo = session.config().combo;
    for (side, sig) in [(&sides[0], a), (&sides[1], b)] {
        match cleartext_file_size(sig.size, combo) {
            Ok(size) if size <= MERGE_MAX_BYTES => {}
            _ => return false,
        }
        match read_clear(side, &conflict.rel, session) {
            Ok(bytes) if std::str::from_utf8(&bytes).is_ok() => {}
            _ => return false,
        }
    }
    true
}

/// The conflict of the entry `item`, when it offers the merge.
fn mergeable_conflict(state: &CompareState, item: &str) -> io::Result<Conflict> {
    state.check_unlocked()?;
    let items = state.items();
    let held = items
        .iter()
        .find(|held| held.item.id == item && !held.done)
        .ok_or_else(super::compare_cache::stale)?;
    if !held.item.actions.contains(&ItemAction::Merge) {
        return Err(invalid("Dieser Eintrag lässt sich nicht zusammenführen"));
    }
    match &held.payload {
        Payload::Files { conflicts, .. } if conflicts.len() == 1 => Ok(conflicts[0].clone()),
        _ => Err(invalid("Dieser Eintrag lässt sich nicht zusammenführen")),
    }
}

fn utf8(bytes: &[u8]) -> io::Result<Zeroizing<String>> {
    std::str::from_utf8(bytes)
        .map(|text| Zeroizing::new(text.to_string()))
        .map_err(|_| invalid("Die Datei ist kein gültiger UTF-8-Text"))
}

/// The two versions of a mergeable entry as text (`CompareText`): cleartext
/// only in the memory of this call and of the caller.
pub(crate) fn compare_text_job(
    job: &str,
    item: &str,
) -> io::Result<(Zeroizing<String>, Zeroizing<String>)> {
    let state = cache().get(&super::compare_actions::state_key(job))?;
    let conflict = mergeable_conflict(&state, item)?;
    let a = read_clear(&state.sides[0], &conflict.rel, &state.session)?;
    let b = read_clear(&state.sides[1], &conflict.rel, &state.session)?;
    Ok((utf8(&a)?, utf8(&b)?))
}

/// Resolves the conflict of `item` with the merged text `content`
/// (`MergeApply`): the engine replaces both sides, keeps both originals as
/// versions and records the state.
pub(crate) fn merge_apply_item(
    job: &str,
    item: &str,
    content: &str,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let state = cache().get(&super::compare_actions::state_key(job))?;
    let conflict = mergeable_conflict(&state, item)?;
    if u64::try_from(content.len()).unwrap_or(u64::MAX) > MERGE_MAX_BYTES {
        return Err(invalid("Der zusammengeführte Text ist zu groß"));
    }
    let (a, b) = (&state.sides[0], &state.sides[1]);
    let original_a = cipher_bytes(a, &conflict.rel, &state.session)?;
    let original_b = cipher_bytes(b, &conflict.rel, &state.session)?;
    let merged = encrypt_bytes(&state.session.cryptor(), content.as_bytes())
        .map_err(|error| state.session.format_error(error))?;
    {
        let preview = state
            .preview
            .as_ref()
            .ok_or_else(|| invalid("Der Vergleich gehört zu keinem Tresor-Job"))?
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = preview
            .preview
            .state
            .as_ref()
            .ok_or_else(|| invalid("Die Vorschau hat keinen Zustandsbezug"))?;
        merge_recorded_for_key(
            a.backend.as_ref(),
            &a.root,
            b.backend.as_ref(),
            &b.root,
            key,
            &conflict,
            OriginalContent {
                signature: conflict.a,
                bytes: Some(&original_a),
            },
            OriginalContent {
                signature: conflict.b,
                bytes: Some(&original_b),
            },
            MergeChoice::Write(&merged),
            cancel,
            |_phase| (),
        )
        .map_err(|failure| failure.error)?;
    }
    super::compare_actions::mark_done(&state, item);
    Ok(())
}
