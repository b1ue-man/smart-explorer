//! The Share host's walk next to the data between two real Iroh nodes on
//! loopback: with `meta` every entry carries the attributes of its listing
//! (links and special files as entries, folder sizes as listed), without it
//! the former items; folders come before their content although the host
//! reads on several threads; a host without `hash_walk_meta_v1` is never
//! asked for attributes; and a sync tree read through the host equals the
//! one read from the same folder locally.
use std::collections::HashMap;
use std::fs;
use std::io;
use std::sync::atomic::AtomicBool;

use super::CopyPastePeerFixture;
use crate::vfs::{self as vfs, HashWalkItem, HashWalkRequest, OmissionReason};

fn walk(
    fixture: &CopyPastePeerFixture,
    request: HashWalkRequest,
) -> io::Result<(bool, Vec<HashWalkItem>)> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let ran = vfs::hash_walk(&*fixture.peer, "/A", request, tx, &AtomicBool::new(false))?;
    Ok((ran, rx.try_iter().collect()))
}

fn plain(meta: bool) -> HashWalkRequest {
    HashWalkRequest {
        algorithm: None,
        min_bytes: 0,
        meta,
    }
}

fn tree(fixture: &CopyPastePeerFixture) -> io::Result<()> {
    let root = &fixture.root_a;
    fs::write(root.join("a.txt"), b"abc")?;
    fs::write(root.join(".hidden.txt"), b"h")?;
    for folder in 0..30 {
        let dir = root.join(format!("d{folder}")).join("inner");
        fs::create_dir_all(&dir)?;
        for file in 0..3 {
            fs::write(dir.join(format!("f{file}.txt")), b"12345")?;
        }
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join("a.txt"), root.join("link"))?;
        let fifo = std::ffi::CString::new(root.join("fifo").to_string_lossy().as_bytes())
            .map_err(io::Error::other)?;
        // SAFETY: a valid NUL-terminated path; creates one FIFO node.
        if unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[test]
fn abgleich_task_share_host_walk_meta_entries_match_the_listing() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    tree(&fixture)?;
    let (ran, items) = walk(&fixture, plain(true))?;
    assert!(ran);
    let listed: HashMap<String, vfs::VfsMeta> = vfs::list_dir_tolerant(&*fixture.peer, "/A")?
        .entries
        .into_iter()
        .map(|meta| (meta.name.clone(), meta))
        .collect();
    let mut seen = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        let HashWalkItem::Entry(entry) = item else {
            panic!("no omission expected: {item:?}");
        };
        let meta = entry.meta.expect("every entry carries attributes");
        if let Some((parent, _)) = entry.rel.rsplit_once('/') {
            assert!(seen.contains_key(parent), "{parent} after {}", entry.rel);
        } else {
            let row = &listed[&entry.rel];
            assert_eq!(
                (
                    row.is_dir,
                    row.is_symlink,
                    row.special,
                    row.hidden,
                    row.size
                ),
                (
                    entry.is_dir,
                    meta.is_symlink,
                    meta.special,
                    meta.hidden,
                    entry.size
                ),
                "{}",
                entry.rel
            );
        }
        seen.insert(entry.rel.clone(), index);
    }
    assert_eq!(seen.len(), 2 + 30 * 5 + if cfg!(unix) { 2 } else { 0 });
    Ok(())
}

#[test]
fn abgleich_task_share_host_walk_without_meta_keeps_the_former_items() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    tree(&fixture)?;
    let (ran, items) = walk(&fixture, plain(false))?;
    assert!(ran);
    let mut omitted = Vec::new();
    let mut entries = 0;
    for item in items {
        match item {
            HashWalkItem::Entry(entry) => {
                entries += 1;
                assert!(entry.meta.is_none() && entry.digest.is_none());
                if entry.is_dir {
                    assert_eq!(entry.size, 0);
                }
            }
            HashWalkItem::Omitted(omission) => omitted.push((omission.rel, omission.reason)),
        }
    }
    assert_eq!(entries, 2 + 30 * 5);
    omitted.sort_by(|a, b| a.0.cmp(&b.0));
    if cfg!(unix) {
        assert_eq!(
            omitted,
            [
                ("fifo".to_string(), OmissionReason::Special),
                ("link".to_string(), OmissionReason::Link)
            ]
        );
    }
    Ok(())
}

#[test]
fn abgleich_task_share_host_without_meta_capability_is_not_asked() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    fs::write(fixture.root_a.join("a.txt"), b"abc")?;
    // Learn the host's features first, then pose as a host before this change.
    assert!(walk(&fixture, plain(false))?.0);
    if let Ok(mut slot) = fixture.peer.transfer.features.lock() {
        if let Some((_, features)) = slot.as_mut() {
            features.hash_walk_meta_v1 = false;
        }
    }
    let (ran, items) = walk(&fixture, plain(true))?;
    assert!(!ran && items.is_empty());
    assert!(walk(&fixture, plain(false))?.0);
    Ok(())
}

#[test]
fn abgleich_task_share_sync_tree_equals_the_local_tree() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    tree(&fixture)?;
    let globs = crate::bisync::empty_globset();
    let filter = crate::bisync::WalkFilter::basic(true, &globs);
    let cancel = AtomicBool::new(false);
    let mode = crate::bisync::HashMode::FullFresh;
    let remote = crate::bisync::walk_files(&*fixture.peer, "/A", &cancel, &filter, mode, None)?;
    let root = fixture.root_a.to_string_lossy().replace('\\', "/");
    let local = vfs::LocalBackend::new(&root);
    let here = crate::bisync::walk_files(&local, &root, &cancel, &filter, mode, None)?;
    assert_eq!(remote, here);
    assert!(remote.contains_key("d7/inner/f2.txt"));
    Ok(())
}

#[test]
fn abgleich_task_share_packets_carry_the_named_time() -> io::Result<()> {
    use crate::vfs::{Backend, BatchPut, BatchPutOutcome};
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let peer = &*fixture.peer;
    let put = |name: &str, data: &[u8]| -> io::Result<()> {
        let entries = [BatchPut {
            path: format!("/A/{name}"),
            size: data.len() as u64,
            mtime_ms: Some(1_500_000_000_000),
        }];
        let mut data = data;
        let outcomes = peer.put_batch(&entries, &mut data)?;
        assert!(matches!(outcomes[0], BatchPutOutcome::Published(_)));
        Ok(())
    };
    let millis = |name: &str| -> io::Result<u128> {
        let modified = fs::metadata(fixture.root_a.join(name))?.modified()?;
        Ok(modified
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_millis())
    };
    put("timed.txt", b"abc")?;
    assert!(vfs::batch_put_times(peer));
    assert_eq!(millis("timed.txt")?, 1_500_000_000_000);
    Ok(())
}

#[test]
fn abgleich_task_share_wire_stays_additive() {
    use crate::share::wire::{FsBatchPut, FsHashWalk, FsHostFeatures};
    let entry = FsBatchPut {
        path: "/A/x".into(),
        size: 1,
        mtime_ms: None,
    };
    assert!(!serde_json::to_string(&entry).unwrap().contains("mtime_ms"));
    let old: FsBatchPut = serde_json::from_str(r#"{"path":"/A/x","size":1}"#).unwrap();
    assert_eq!(old, entry);
    let walk: FsHashWalk = serde_json::from_str(r#"{"path":"/A","min_bytes":0}"#).unwrap();
    assert!(!walk.meta);
    assert!(!serde_json::to_string(&walk).unwrap().contains("meta"));
    let old_host: FsHostFeatures = serde_json::from_str(r#"{"hash_walk_v1":true}"#).unwrap();
    assert!(old_host.hash_walk_v1 && !old_host.hash_walk_meta_v1);
    assert!(FsHostFeatures::host()
        .names()
        .contains(&"hash_walk_meta_v1"));
}

/// The host's walk with attributes sends each folder as one unbroken run,
/// also across threads and for a folder that fills several batches, with
/// entries and omissions (names that are no UTF-8) alternating.
#[cfg(unix)]
#[test]
fn abgleich_task_share_host_meta_walk_sends_each_folder_as_one_run() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    crate::agent::walk_runs_check::run_tree(&fixture.root_a);
    let (ran, items) = walk(&fixture, plain(true))?;
    assert!(ran);
    let runs = crate::agent::walk_runs_check::assert_folder_runs(&items);
    assert_eq!(runs, 2 + 40 * 2);
    let omitted = items
        .iter()
        .filter(|item| matches!(item, HashWalkItem::Omitted(_)))
        .count();
    assert_eq!(omitted, 5 + 40);
    assert_eq!(items.len() - omitted, 1 + 3000 + 40 * (1 + 1 + 3));
    Ok(())
}

/// W6: a subfolder the host cannot open is its entry, then an omission of
/// the same path, as the folder walk shows its row and then its failure.
#[cfg(unix)]
#[test]
fn abgleich_task_share_host_walk_rows_an_unopenable_folder() -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let locked = fixture.root_a.join("locked");
    fs::create_dir(&locked)?;
    fs::write(locked.join("inside.txt"), b"x")?;
    fs::write(fixture.root_a.join("a.txt"), b"abc")?;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000))?;
    // A process that may read every folder (root) cannot show the failure.
    let readable = fs::read_dir(&locked).is_ok();
    let walked = (|| -> io::Result<()> {
        if readable {
            return Ok(());
        }
        for meta in [true, false] {
            let (ran, items) = walk(&fixture, plain(meta))?;
            assert!(ran);
            let entry = items
                .iter()
                .position(|item| {
                    matches!(item, HashWalkItem::Entry(entry) if entry.rel == "locked" && entry.is_dir)
                })
                .expect("the folder's row");
            let omission = items
                .iter()
                .position(|item| {
                    matches!(item, HashWalkItem::Omitted(hole)
                        if hole.rel == "locked" && hole.reason == OmissionReason::Unreadable)
                })
                .expect("the folder's failure");
            assert!(entry < omission, "row before failure (meta = {meta})");
            assert!(!items.iter().any(|item| match item {
                HashWalkItem::Entry(entry) => entry.rel.starts_with("locked/"),
                HashWalkItem::Omitted(hole) => hole.rel.starts_with("locked/"),
            }));
            if meta {
                crate::agent::walk_runs_check::assert_folder_runs(&items);
            }
        }
        Ok(())
    })();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755))?;
    walked
}

/// W5: a stage written with the source's time (`stage_mtime_v1`) gets it at
/// its commit, before the host reads its evidence, so the evidence shows it
/// and publishing sends no `FinishStage`.
#[test]
fn abgleich_task_share_stage_carries_the_source_time_with_its_write() -> io::Result<()> {
    use std::io::Write;
    const TIME: i64 = 1_500_000_000_000;
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let peer = &*fixture.peer;
    let name = "timed.txt.se-upload-00000000000000aa";
    let stage = format!("/A/{name}");
    let mut writer = vfs::open_write_copy_stage_timed(peer, &stage, 3, TIME)?;
    writer.write_all(b"abc")?;
    writer.flush()?;
    drop(writer);
    let evidence = vfs::take_stage_evidence(peer, &stage).expect("the host's evidence");
    assert_eq!((evidence.stage.size, evidence.stage.mtime_ms), (3, TIME));
    let modified = fs::metadata(fixture.root_a.join(name))?.modified()?;
    let millis = modified
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_millis();
    assert_eq!(millis, TIME as u128);
    Ok(())
}
