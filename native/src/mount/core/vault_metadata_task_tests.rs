//! Host-cache acceptance for the one remote mount_vault_task suite.
use super::{order, Admission, DirectoryObservation, MetadataCache, MetadataChange,
    MetadataLookup, MAX_CACHED_BYTES};
use crate::mount::metadata_point_cache::MetadataPointCache;
use crate::vfs::VfsMeta;
use std::{io, sync::Arc, time::{Duration, Instant}};

pub(super) fn directory(name: &str) -> VfsMeta {
    VfsMeta { name: name.into(), is_dir: true, id: Some(format!("dir-{name}")),
        ..VfsMeta::default() }
}

pub(super) fn file(name: &str, generation: i64) -> VfsMeta {
    VfsMeta { name: name.into(), size: 4, mtime_ms: generation,
        id: Some(format!("file-{name}")), ..VfsMeta::default() }
}

pub(super) fn observation(entries: Vec<VfsMeta>) -> DirectoryObservation {
    let expires = Instant::now() + super::DIRECTORY_TTL;
    DirectoryObservation { metadata: directory("/"), metadata_expires_at: expires,
        entries: entries.into(), listing_expires_at: expires }
}

impl MetadataCache {
    pub(in crate::mount) fn test_expire_directory(&self, path: &str) -> io::Result<()> {
        let mut state = self.lock_state()?;
        let key = self.key(path);
        assert!(state.directories.contains_key(&key), "fixture must have a baseline");
        order::expire(&mut state, &key, Instant::now() - Duration::from_secs(1));
        Ok(())
    }

    pub(in crate::mount) fn test_change_budget(&self, bytes: Option<usize>) -> io::Result<usize> {
        Ok(self.lock_state()?.changes.set_test_byte_budget(bytes))
    }

    // Model a full retained cache without allocating 128 MiB. Accounting and
    // the victim image agree; admission/eviction still use production paths.
    pub(in crate::mount) fn test_fill_retention(&self, path: &str) -> io::Result<()> {
        let mut state = self.lock_state()?;
        assert_eq!(state.cooldown_bytes, 0);
        let extra = MAX_CACHED_BYTES - state.bytes;
        state.directories.get_mut(&self.key(path)).expect("fixture snapshot").byte_count += extra;
        state.bytes += extra;
        Ok(())
    }
}

#[test]
fn mount_vault_task_root_is_not_its_own_descendant() -> io::Result<()> {
    let paths = ["/", "/a", "/a/deep", "/ab"].into_iter()
        .map(|path| (path.to_string(), ())).collect();
    assert_eq!(order::descendants(&paths, "/"), vec!["/a", "/a/deep", "/ab"]);
    assert_eq!(order::descendants(&paths, "/a"), vec!["/a/deep"]);
    let cache = MetadataCache::new("/", true);
    let points = MetadataPointCache::new(true);
    let root = cache.load_slot("/")?;
    let child = cache.load_slot("/a")?;
    let root_revision = root.revision();
    let child_revision = child.revision();
    assert!(cache.install_observation("/", observation(vec![directory("a")]),
        0, Some((&root, root_revision)), Admission::Demand)?);
    assert_eq!(root.revision(), root_revision.wrapping_add(1));
    assert_eq!(child.revision(), child_revision.wrapping_add(1));
    let entries = cache.directory("/")?.expect("root authority survives reconciliation");
    root.complete_directory(root.revision(), Instant::now() + super::DIRECTORY_TTL,
        Arc::clone(&entries))?;
    assert!(Arc::ptr_eq(&entries, &root.completed_directory()?.unwrap()));

    let root_revision = root.revision();
    let child_revision = child.revision();
    assert!(cache.install_point_if_current("/", &root, root_revision,
        &points, Some(directory("/")))?);
    assert_eq!(root.revision(), root_revision.wrapping_add(1));
    assert_eq!(child.revision(), child_revision.wrapping_add(1));
    assert!(root.completed_directory()?.is_none());
    assert!(matches!(points.lookup("/")?, MetadataLookup::Found(_)));
    assert!(cache.directory("/")?.is_none(), "exact point expires the old listing");
    Ok(())
}

#[test]
fn mount_vault_task_more_than_4096_small_directories_remain_reusable() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    let count = 4_200;
    assert!(cache.install_observation("/", observation((0..count)
        .map(|index| directory(&format!("d{index}"))).collect()), 0, None, Admission::Demand)?);
    for index in 0..count {
        assert!(cache.install_observation(&format!("/d{index}"),
            observation(vec![file("note.md", 1)]), 1, None, Admission::Demand)?);
    }
    assert_eq!(cache.usage()?.0, count + 1);
    assert!(cache.usage()?.2 < MAX_CACHED_BYTES);
    for index in 0..count {
        let path = format!("/d{index}");
        let first = cache.directory(&path)?.expect("under-budget snapshot retained");
        assert!(Arc::ptr_eq(&first, &cache.directory(&path)?.unwrap()));
        assert!(matches!(cache.stat(&format!("{path}/absent"))?, MetadataLookup::KnownMissing));
    }
    Ok(())
}

#[test]
fn mount_vault_task_unchanged_parent_preserves_child_and_replacement_invalidates() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    let points = MetadataPointCache::new(true);
    let parent = vec![directory("a"), directory("ab")];
    assert!(cache.install_observation("/", observation(parent.clone()), 0, None, Admission::Demand)?);
    assert!(cache.install_observation("/a", observation(vec![directory("deep")]),
        1, None, Admission::Demand)?);
    assert!(cache.install_observation("/a/deep", observation(vec![file("note", 1)]),
        2, None, Admission::Demand)?);
    assert!(cache.install_observation("/ab", observation(vec![file("safe", 1)]),
        1, None, Admission::Demand)?);
    let child = cache.load_slot("/a")?;
    let nested = cache.load_slot("/a/deep")?;
    let unrelated = cache.load_slot("/ab")?;
    let child_revision = child.revision();
    let nested_revision = nested.revision();
    let unrelated_revision = unrelated.revision();
    let image = cache.directory("/a/deep")?.unwrap();
    points.install("/a/deep/point", file("point", 1))?;
    points.install("/ab/point", file("point", 1))?;
    assert!(cache.install_observation_reconciled("/", observation(parent.clone()),
        0, None, Admission::Refresh, &points)?);
    assert_eq!(child.revision(), child_revision);
    assert_eq!(nested.revision(), nested_revision);
    assert!(Arc::ptr_eq(&image, &cache.directory("/a/deep")?.unwrap()));
    assert!(cache.drain_changes(20)?.is_empty());

    let mut replaced = parent;
    replaced[0].id = Some("replacement-a".into());
    assert!(cache.install_observation_reconciled("/", observation(replaced),
        0, None, Admission::Refresh, &points)?);
    assert_ne!(child.revision(), child_revision);
    assert_ne!(nested.revision(), nested_revision);
    assert_eq!(unrelated.revision(), unrelated_revision);
    assert!(cache.directory("/a/deep")?.is_none());
    assert!(matches!(points.lookup("/a/deep/point")?, MetadataLookup::Uncached));
    assert!(matches!(points.lookup("/ab/point")?, MetadataLookup::Found(_)));
    assert!(!cache.install_observation("/a/deep", observation(vec![file("stale", 1)]),
        2, Some((&nested, nested_revision)), Admission::Demand)?);
    assert!(cache.directory("/ab")?.is_some());

    let removed_revision = child.revision();
    assert!(cache.install_observation_reconciled("/", observation(vec![directory("ab")]),
        0, None, Admission::Refresh, &points)?);
    assert_ne!(child.revision(), removed_revision);
    assert!(matches!(cache.stat("/a/deep/note")?, MetadataLookup::KnownMissing));
    assert_eq!(unrelated.revision(), unrelated_revision);
    Ok(())
}

#[test]
fn mount_vault_task_byte_lru_can_evict_root_without_evicting_speculatively() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    assert!(cache.install_observation("/", observation(vec![directory("child")]),
        0, None, Admission::Demand)?);
    cache.test_fill_retention("/")?;
    let before = cache.usage()?;
    let root_revision = cache.revision("/")?;
    assert!(!cache.install_observation("/child", observation(vec![file("note", 1)]),
        1, None, Admission::Speculative)?);
    assert_eq!(cache.usage()?, before);
    assert_eq!(cache.revision("/")?, root_revision);
    assert!(cache.install_observation("/child", observation(vec![file("note", 1)]),
        1, None, Admission::Demand)?);
    assert!(cache.revision("/")?.is_none(), "root is ordinary byte-LRU, not permanently pinned");
    assert!(cache.directory("/child")?.is_some());
    assert!(cache.usage()?.2 < MAX_CACHED_BYTES);
    Ok(())
}

#[test]
fn mount_vault_task_notification_byte_pressure_retains_baseline_and_retries() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    assert!(cache.install_observation("/", observation(vec![directory("a"), directory("b")]),
        0, None, Admission::Demand)?);
    for path in ["/a", "/b"] {
        assert!(cache.install_observation(path, observation(vec![file("note", 1)]),
            1, None, Admission::Demand)?);
    }
    assert!(cache.install_observation("/a", observation(vec![file("note", 2)]),
        1, None, Admission::Refresh)?);
    let first_image_bytes = cache.test_change_budget(None)?;
    assert!(first_image_bytes > 0 && first_image_bytes < 32 * 1024);
    cache.test_change_budget(Some(first_image_bytes))?;
    let old_revision = cache.revision("/b")?;
    let child = cache.load_slot("/b/note")?;
    let child_revision = child.revision();
    assert!(!cache.install_observation("/b", observation(vec![file("note", 2)]),
        1, None, Admission::Refresh)?);
    assert_eq!(cache.revision("/b")?, old_revision);
    assert_eq!(child.revision(), child_revision);
    let MetadataLookup::Found(old) = cache.stat("/b/note")? else { panic!("lost baseline"); };
    assert_eq!(old.mtime_ms, 1);
    assert_eq!(cache.drain_changes(20)?, vec![MetadataChange::Modified { path: "/a/note".into() }]);
    assert!(cache.install_observation("/b", observation(vec![file("note", 2)]),
        1, None, Admission::Refresh)?);
    assert_eq!(cache.drain_changes(20)?, vec![MetadataChange::Modified { path: "/b/note".into() }]);
    assert_eq!(cache.test_change_budget(None)?, 0);
    // The override belongs only to this cache instance.
    cache.test_change_budget(Some(0))?;
    let independent = MetadataCache::new("/", true);
    assert!(independent.install_observation("/", observation(vec![file("note", 1)]),
        0, None, Admission::Demand)?);
    assert!(independent.install_observation("/", observation(vec![file("note", 2)]),
        0, None, Admission::Refresh)?);
    Ok(())
}

#[test]
fn mount_vault_task_bounded_diff_comparison_resumes_after_empty_drain() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    let old = (0..5_000).map(|index| file(&format!("f{index:05}"), 1)).collect::<Vec<_>>();
    let mut new = old.clone();
    new[4_999].mtime_ms = 2;
    assert!(cache.install_observation("/", observation(old), 0, None, Admission::Demand)?);
    assert!(cache.install_observation("/", observation(new.clone()), 0, None, Admission::Refresh)?);
    assert!(cache.drain_changes(usize::MAX)?.is_empty(), "one call compares at most 4096 records");
    // Once comparisons have started, an additional commit cannot reset progress
    // by replacing the tail, even if the preceding drain emitted no event.
    new[4_999].mtime_ms = 3;
    assert!(cache.install_observation("/", observation(new), 0, None, Admission::Refresh)?);
    let mut concrete = Vec::new();
    for _ in 0..10 { concrete.extend(cache.drain_changes(usize::MAX)?); }
    assert_eq!(concrete, vec![MetadataChange::Modified { path: "/f04999".into() }; 2]);
    assert_eq!(cache.test_change_budget(None)?, 0);
    Ok(())
}
