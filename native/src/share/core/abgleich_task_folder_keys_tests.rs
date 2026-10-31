//! Folder keys of the Share host's walk (E10, `hash_walk_keys_v1`) between
//! two real Iroh nodes on loopback: every folder's run begins with its mark
//! and ends with its key, the host's key is the key the client computes
//! from the sent items, a known tree comes as one mark without any item, a
//! changed file sends only the folders on its path, and a host without the
//! capability is never asked for keys.
use std::collections::HashMap;
use std::fs;
use std::io;
use std::sync::atomic::AtomicBool;

use super::CopyPastePeerFixture;
use crate::agent_proto::{entry_part, folder_key, known_prefix, run_digest, EntryKind, FolderKey};
use crate::vfs::{self as vfs, HashWalkItem, HashWalkRequest, KeyedWalkItem};

fn keyed(
    fixture: &CopyPastePeerFixture,
    known: &[[u8; 16]],
) -> io::Result<(bool, Vec<KeyedWalkItem>)> {
    let request = HashWalkRequest {
        algorithm: Some(crate::analytics::HashAlgorithm::Md5),
        min_bytes: 0,
        meta: true,
        fresh: false,
        filter: None,
    };
    let (tx, rx) = crossbeam_channel::unbounded();
    let cancel = AtomicBool::new(false);
    let ran = vfs::hash_walk_keyed(
        &*fixture.peer,
        "/A",
        request,
        vfs::KeyedWalk::Marks { known },
        tx,
        &cancel,
    )?;
    Ok((ran, rx.try_iter().collect()))
}

fn tree(fixture: &CopyPastePeerFixture) -> io::Result<()> {
    let root = &fixture.root_a;
    fs::write(root.join("a.txt"), b"abc")?;
    fs::create_dir_all(root.join("leer"))?;
    for folder in 0..5 {
        let dir = root.join(format!("d{folder}"));
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("f.txt"), format!("inhalt {folder}"))?;
    }
    Ok(())
}

fn keys(items: &[KeyedWalkItem]) -> HashMap<String, FolderKey> {
    items
        .iter()
        .filter_map(|item| match item {
            KeyedWalkItem::Key { rel, key, .. } => Some((rel.clone(), *key)),
            _ => None,
        })
        .collect()
}

fn runs(items: &[KeyedWalkItem]) -> Vec<String> {
    let mut runs: Vec<String> = items
        .iter()
        .filter_map(|item| match item {
            KeyedWalkItem::Run { rel } => Some(rel.clone()),
            _ => None,
        })
        .collect();
    runs.sort();
    runs
}

#[test]
fn abgleich_task_share_host_keys_every_folder_as_the_client_computes_it() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    tree(&fixture)?;
    let (ran, items) = keyed(&fixture, &[])?;
    assert!(ran);
    assert_eq!(runs(&items), ["", "d0", "d1", "d2", "d3", "d4", "leer"]);
    let keys = keys(&items);
    assert_eq!(keys.len(), 7);
    // The run of `d2` is its mark, then one file with its MD5.
    let start = items
        .iter()
        .position(|item| matches!(item, KeyedWalkItem::Run { rel } if rel == "d2"))
        .unwrap();
    let KeyedWalkItem::Item(HashWalkItem::Entry(entry)) = &items[start + 1] else {
        panic!("no entry after the run mark: {:?}", items[start + 1]);
    };
    assert_eq!(entry.rel, "d2/f.txt");
    let part = entry_part(
        "f.txt",
        EntryKind::File,
        entry.size,
        entry.mtime_ms,
        entry.digest.as_deref(),
        false,
    );
    let expected = folder_key(&run_digest(vec![part]), Vec::new());
    assert_eq!(keys["d2"], expected, "host and client form the same key");
    // The root's key comes after everything else.
    assert!(
        matches!(items.last(), Some(KeyedWalkItem::Key { rel, same: false, .. }) if rel.is_empty())
    );
    Ok(())
}

#[test]
fn abgleich_task_share_host_answers_known_folders_with_one_mark() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    tree(&fixture)?;
    let (_, first) = keyed(&fixture, &[])?;
    let known: Vec<[u8; 16]> = keys(&first).values().map(known_prefix).collect();
    let (ran, second) = keyed(&fixture, &known)?;
    assert!(ran);
    assert_eq!(second.len(), 1, "{second:?}");
    assert!(matches!(&second[0], KeyedWalkItem::Key { rel, same: true, .. } if rel.is_empty()));
    // One file changes: the root and its folder are sent, the rest are marks.
    fs::write(
        fixture.root_a.join("d3").join("f.txt"),
        b"ganz anderer inhalt",
    )?;
    let (_, third) = keyed(&fixture, &known)?;
    assert_eq!(runs(&third), ["", "d3"]);
    let same: Vec<&str> = third
        .iter()
        .filter_map(|item| match item {
            KeyedWalkItem::Key {
                rel, same: true, ..
            } => Some(rel.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(same.len(), 5, "d0, d1, d2, d4 and leer: {same:?}");
    let entries = third
        .iter()
        .filter(|item| matches!(item, KeyedWalkItem::Item(_)))
        .count();
    assert_eq!(entries, 8, "the root's seven items and d3's file");
    Ok(())
}

#[test]
fn abgleich_task_share_host_without_keys_capability_is_not_asked() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    fs::write(fixture.root_a.join("a.txt"), b"abc")?;
    assert!(keyed(&fixture, &[])?.0);
    if let Ok(mut slot) = fixture.peer.transfer.features.lock() {
        if let Some((_, features)) = slot.as_mut() {
            features.hash_walk_keys_v1 = false;
        }
    }
    let (ran, items) = keyed(&fixture, &[])?;
    assert!(!ran && items.is_empty());
    Ok(())
}

/// The keys-only phase sends one key per folder with what its subtree holds
/// and no item (`vfs::KeyTally`): the caller settles an equal folder from it
/// without a listing.
#[test]
fn abgleich_task_share_host_keys_only_counts_each_subtree_and_sends_no_item() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    tree(&fixture)?;
    let request = HashWalkRequest {
        algorithm: None,
        min_bytes: 0,
        meta: true,
        fresh: false,
        filter: None,
    };
    let (tx, rx) = crossbeam_channel::unbounded();
    let ran = vfs::hash_walk_keyed(
        &*fixture.peer,
        "/A",
        request,
        vfs::KeyedWalk::KeysOnly,
        tx,
        &AtomicBool::new(false),
    )?;
    assert!(ran);
    let items: Vec<KeyedWalkItem> = rx.try_iter().collect();
    let mut tallies = HashMap::new();
    for item in &items {
        match item {
            KeyedWalkItem::Key {
                rel,
                tally: Some(tally),
                same: false,
                ..
            } => {
                tallies.insert(rel.clone(), *tally);
            }
            other => panic!("keys only sends tallied keys, got {other:?}"),
        }
    }
    assert_eq!(tallies.len(), 7, "root, leer and d0..d4");
    let root = tallies[""];
    // a.txt (3 bytes) and five files of "inhalt N" (8 bytes each).
    assert_eq!((root.files, root.bytes, root.height), (6, 43, 1));
    assert!(root.clean(), "{root:?}");
    assert_eq!((tallies["leer"].files, tallies["leer"].height), (0, 0));
    assert_eq!((tallies["d3"].files, tallies["d3"].bytes), (1, 8));
    Ok(())
}
