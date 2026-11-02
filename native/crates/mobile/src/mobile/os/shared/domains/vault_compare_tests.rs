//! VLT-M11 (U31-17, S2): pages of the comparison and the text of a failed
//! vault task outside the protected screens.
use super::*;
use crate::vault::{ItemAction, ItemKind, LockReason, RegisterEvent, SideState, SiteState};

fn item(index: usize, group: CompareGroup) -> CompareItem {
    let side = |exists: bool| SideState {
        exists,
        size: exists.then_some(10 + index as u64),
        mtime_ms: exists.then_some(1_760_000_000_000),
        location: "Privat".to_string(),
    };
    CompareItem {
        id: format!("{index:016x}"),
        group,
        kind: ItemKind::File,
        path: format!("/Ordner/Geheim {index}.txt"),
        a: side(true),
        b: side(index % 2 == 0),
        actions: vec![ItemAction::KeepA, ItemAction::KeepB],
        finding: None,
        note: None,
    }
}

/// U31-17: entries of one group page by page, no fixed page size; without
/// `limit` every entry from `offset` on.
#[test]
fn vault_task_android_compare_items_pages_by_group() {
    let mut listing: Vec<CompareItem> = (0..700)
        .map(|index| item(index, CompareGroup::Conflict))
        .collect();
    listing.extend((700..710).map(|index| item(index, CompareGroup::NewA)));
    let first = page(&listing, Some(CompareGroup::Conflict), 0, Some(600));
    assert_eq!(first["items"].as_array().unwrap().len(), 600);
    assert_eq!(first["nextOffset"], 600);
    assert_eq!(first["total"], 700);
    assert_eq!(first["items"][0]["path"], "/Ordner/Geheim 0.txt");
    assert_eq!(first["items"][0]["a"]["mtimeMs"], 1_760_000_000_000_i64);
    assert_eq!(first["items"][0]["actions"], json!(["keep-a", "keep-b"]));
    let rest = page(&listing, Some(CompareGroup::Conflict), 600, None);
    assert_eq!(rest["items"].as_array().unwrap().len(), 100);
    assert!(rest["nextOffset"].is_null());
    let new_a = page(&listing, Some(CompareGroup::NewA), 0, None);
    assert_eq!(new_a["total"], 10);
    assert_eq!(new_a["items"][0]["group"], "new_a");
    assert_eq!(page(&listing, None, 0, None)["total"], 710);
    let beyond = page(&listing, Some(CompareGroup::NewA), 50, Some(5));
    assert!(beyond["items"].as_array().unwrap().is_empty());
}

/// S2: a failed task reports no file name to the error log or the
/// notification; a cancellation and a locked vault keep their texts.
#[test]
fn vault_task_android_failed_task_text_has_no_names() {
    let failed = ApiError::new("conflict", "Geheim 7.txt: Zieldatei geändert");
    assert_eq!(public_text(&failed), FAILED_TEXT);
    assert!(!public_text(&failed).contains("Geheim"));
    let canceled = ApiError::canceled();
    assert_eq!(public_text(&canceled), canceled.message);
    let locked = ApiError::new(crate::vault::VAULT_LOCKED_CODE, "Tresor ist gesperrt");
    assert_eq!(public_text(&locked), "Tresor ist gesperrt");
}

/// U31-17: the `vault` event names site, state and one of the five reason
/// keys of the screens.
#[test]
fn vault_task_android_vault_event_shape() {
    let event = |state, reason| RegisterEvent {
        seq: 1,
        site: "0123456789abcdef0123456789abcdef".to_string(),
        state,
        reason,
        epoch: 3,
    };
    assert_eq!(
        super::super::vaults::event_json(&event(SiteState::Locked, Some(LockReason::ScreenOff))),
        json!({"type": "vault", "site": "0123456789abcdef0123456789abcdef",
               "state": "locked", "reason": "screen_off"})
    );
    let unlocked = super::super::vaults::event_json(&event(SiteState::Unlocked, None));
    assert_eq!(unlocked["state"], "unlocked");
    assert!(unlocked["reason"].is_null());
    let keys = [
        (LockReason::Background, Some("background")),
        (LockReason::Idle, Some("idle")),
        (LockReason::Explicit, Some("manual")),
        (LockReason::Removed, Some("manual")),
        (LockReason::WorkerEnd, Some("process_end")),
        (LockReason::Sleep, None),
        (LockReason::MountEnd, None),
    ];
    for (reason, key) in keys {
        assert_eq!(super::super::vaults::reason_key(reason), key, "{reason:?}");
    }
}
