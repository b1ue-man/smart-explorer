//! Incremental cursor and revision-owned selection acceptance, without sleeps or I/O waits.
use super::*;
use super::super::{Admission, RetiredMetadata, SNAPSHOT_RETRY_DELAY};
use super::super::vault_task_tests::{directory, file, observation, observation_at};
use crate::mount::metadata_point_cache::MetadataPointCache;
use crate::vfs::VfsMeta;
use std::{collections::BTreeSet, sync::Arc, time::Duration};

fn selected_paths(batch: &PreloadBatch<'_>) -> Vec<String> {
    batch.tickets().iter().map(|ticket| ticket.path.clone()).collect()
}

fn admit(cache: &MetadataCache, batch: &PreloadBatch<'_>, points: &MetadataPointCache) -> io::Result<()> {
    for ticket in batch.tickets() {
        let slot = cache.load_slot(&ticket.path)?;
        assert!(cache.install_preload_observation(ticket,
            observation_at(&ticket.path, Vec::new()), &slot, slot.revision(), points)?);
    }
    Ok(())
}

fn advance_retry_deadline(cache: &MetadataCache) -> io::Result<()> {
    let mut state = cache.lock_state()?;
    let after = Instant::now() + SNAPSHOT_RETRY_DELAY + Duration::from_secs(1);
    // Advance just retry maintenance; do not expire directory authority or
    // sleep five minutes. Selection still checks real, fresh parent lifetimes.
    order::prune_cooldowns(&mut state, after);
    prune_retries(&mut state, after, cache.case_sensitive);
    Ok(())
}

#[test]
fn mount_vault_task_preload_10000_entry_cursor_survives_cancellation_without_rescan() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    let points = MetadataPointCache::new(true);
    let entries: Arc<[VfsMeta]> = (0..10_000).map(|index| file(&format!("f{index:05}"), 1))
        .chain((0..17).map(|index| directory(&format!("d{index:02}")))).collect::<Vec<_>>().into();
    let mut root = observation(Vec::new());
    root.entries = Arc::clone(&entries);
    assert!(cache.install_observation("/", root, 0, None, Admission::Demand)?);
    for expected_cursor in [DISCOVERY_BUDGET, 2 * DISCOVERY_BUDGET] {
        let batch = cache.select_preload(2, 8)?;
        assert!(batch.tickets().is_empty(), "discovered files are not directory work");
        assert_eq!(cache.lock_state()?.preload.cursor(2).unwrap().1, expected_cursor);
        assert!(batch.finish()?, "undiscovered positions prevent a false idle result");
    }
    let canceled = cache.select_preload(2, 8)?;
    assert_eq!(canceled.tickets().len(), 8);
    let expected = selected_paths(&canceled);
    let cursor = cache.lock_state()?.preload.cursor(2).unwrap().1;
    assert_eq!(cursor, 10_008);
    assert_eq!(Arc::strong_count(&entries), 2, "tickets/queues do not retain snapshot Arcs");
    drop(canceled);
    let resumed = cache.select_preload(2, 8)?;
    assert_eq!(selected_paths(&resumed), expected, "canceled selected work is rearmed exactly once");
    assert_eq!(cache.lock_state()?.preload.cursor(2).unwrap().1, cursor, "retry does not rewind discovery");
    let mut visited = expected.into_iter().collect::<BTreeSet<_>>();
    admit(&cache, &resumed, &points)?;
    assert!(resumed.finish()?);
    let mut finished = false;
    for _ in 0..4 {
        let batch = cache.select_preload(2, 8)?;
        assert!(!batch.tickets().is_empty());
        for path in selected_paths(&batch) { assert!(visited.insert(path), "already admitted child selected twice"); }
        admit(&cache, &batch, &points)?;
        if !batch.finish()? { finished = true; break; }
    }
    assert!(finished, "finite cursor discovery completes without losing work");
    assert_eq!(visited.len(), 17);
    assert_eq!(cache.usage()?.0, 18);
    assert!(cache.lock_state()?.preload.cursor(2).is_none());
    let idle = cache.select_preload(2, 8)?;
    assert!(idle.tickets().is_empty());
    assert!(!idle.finish()?);
    Ok(())
}

#[test]
fn mount_vault_task_preload_old_revision_cannot_retain_or_rearm_replaced_snapshot() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    let points = MetadataPointCache::new(true);
    let entries: Arc<[VfsMeta]> = vec![directory("old")].into();
    let mut root = observation(Vec::new());
    root.entries = Arc::clone(&entries);
    assert!(cache.install_observation("/", root, 0, None, Admission::Demand)?);
    let canceled = cache.select_preload(2, 8)?;
    assert_eq!(selected_paths(&canceled), ["/old"]);
    let slot = cache.load_slot("/old")?;
    let revision = slot.revision();
    cache.invalidate("/", true)?;
    assert_eq!(Arc::strong_count(&entries), 1, "selected tickets cannot pin evicted wide images");
    assert!(cache.install_observation("/", observation(vec![directory("new")]),
        0, None, Admission::Demand)?);
    let old = &canceled.tickets()[0];
    assert!(!cache.preload_ticket_current(old)?);
    assert!(!cache.install_preload_observation(old, observation_at("/old", Vec::new()),
        &slot, revision, &points)?);
    cache.cool_down_preload(old)?;
    assert_eq!(cache.cooldown_count()?, 0, "failed old work cannot cool a replacement parent");
    drop(canceled);
    let current = cache.select_preload(2, 8)?;
    assert_eq!(selected_paths(&current), ["/new"]);
    admit(&cache, &current, &points)?;
    assert!(!current.finish()?);
    assert!(cache.revision("/old")?.is_none());
    Ok(())
}

#[test]
fn mount_vault_task_preload_cooldown_survives_unretained_publication_and_rearms_exact_eviction() -> io::Result<()> {
    let cache = MetadataCache::new("/", false);
    let points = MetadataPointCache::new(false);
    assert!(cache.install_observation("/", observation(vec![directory("A")]),
        0, None, Admission::Demand)?);
    let failed = cache.select_preload(2, 8)?;
    assert_eq!(selected_paths(&failed), ["/A"]);
    cache.cool_down_preload(&failed.tickets()[0])?;
    assert!(!failed.finish()?);
    assert_eq!(cache.cooldown_count()?, 1);
    let cooling = cache.select_preload(2, 8)?;
    assert!(cooling.tickets().is_empty());
    assert!(!cooling.finish()?);
    advance_retry_deadline(&cache)?;

    let rejected = cache.select_preload(2, 8)?;
    assert_eq!(selected_paths(&rejected), ["/A"]);
    let slot = cache.load_slot("/a")?;
    let revision = slot.revision();
    let before = cache.usage()?.2;
    cache.test_fill_retention("/")?;
    assert!(!cache.install_preload_observation(&rejected.tickets()[0], observation_at("/A", Vec::new()),
        &slot, revision, &points)?);
    assert_eq!(slot.revision(), revision.wrapping_add(1), "successful unretained observation still publishes");
    assert_eq!(cache.cooldown_count()?, 0, "no bytes available for a standalone retry hint");
    {
        let mut state = cache.lock_state()?;
        // Reverse only the fixture's artificial byte pressure, so a false idle
        // result cannot be explained by the capacity gate instead of cooldown.
        let extra = state.bytes - before;
        state.directories.get_mut("/").unwrap().byte_count -= extra;
        state.bytes -= extra;
    }
    assert!(!rejected.finish()?, "charged parent record preserves retry after revision advanced");
    let cooling = cache.select_preload(2, 8)?;
    assert!(cooling.tickets().is_empty());
    assert!(!cooling.finish()?);
    advance_retry_deadline(&cache)?;
    let recovered = cache.select_preload(2, 8)?;
    assert_eq!(selected_paths(&recovered), ["/A"]);
    admit(&cache, &recovered, &points)?;
    assert!(!recovered.finish()?);
    assert!(cache.lock_state()?.preload.cursor(2).is_none());
    let mut retired = RetiredMetadata::default();
    {
        let mut state = cache.lock_state()?;
        // Exercise the real eviction hook, not invalidate(), which also removes
        // parent authority. Case-folded lookup must rearm just this child.
        super::super::support::remove_directory(&mut state, "/a", &mut retired);
    }
    drop(retired);
    let evicted = cache.select_preload(2, 8)?;
    assert_eq!(selected_paths(&evicted), ["/A"]);
    assert!(cache.lock_state()?.preload.cursor(2).is_none(), "eviction does not restart discovery");
    admit(&cache, &evicted, &points)?;
    assert!(!evicted.finish()?);
    Ok(())
}
