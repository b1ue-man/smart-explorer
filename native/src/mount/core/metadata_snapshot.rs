//! Snapshot admission/publication; owns the load-table -> state -> points order.
use super::{changes, order, preload, preload_records, support::*,
    Admission, CachedDirectory, DirectoryObservation, LoadSlot, MetadataCache,
    PreloadTicket, RetiredMetadata, SnapshotPublication, MAX_CACHED_DIRECTORY_BYTES};
use super::load_support::publish_observation;
use crate::mount::metadata_point_cache::MetadataPointCache;
use std::io;
use std::sync::Arc;
use std::time::Instant;

impl MetadataCache {
    pub(super) fn install_snapshot(
        &self, path: &str, observation: DirectoryObservation, depth: u8,
        admission: Option<(&LoadSlot, u64)>, intent: Admission,
        points: Option<&MetadataPointCache>,
    ) -> io::Result<bool> {
        self.publish_snapshot(path, observation, depth, admission, intent, points, None)
            .map(|published| published.retained)
    }

    pub(in crate::mount) fn install_observation_publication(
        &self, path: &str, observation: DirectoryObservation, depth: u8,
        slot: &LoadSlot, revision: u64, intent: Admission, points: &MetadataPointCache,
    ) -> io::Result<SnapshotPublication> {
        self.publish_snapshot(path, observation, depth, Some((slot, revision)), intent, Some(points), None)
    }

    pub(in crate::mount) fn install_preload_observation(
        &self, ticket: &PreloadTicket, observation: DirectoryObservation,
        slot: &LoadSlot, revision: u64, points: &MetadataPointCache,
    ) -> io::Result<bool> {
        self.publish_snapshot(&ticket.path, observation, ticket.depth,
            Some((slot, revision)), Admission::Speculative, Some(points), Some(ticket))
            .map(|published| published.retained)
    }

    fn publish_snapshot(
        &self, path: &str, observation: DirectoryObservation, depth: u8,
        admission: Option<(&LoadSlot, u64)>, intent: Admission,
        points: Option<&MetadataPointCache>,
        ticket: Option<&PreloadTicket>,
    ) -> io::Result<SnapshotPublication> {
        let DirectoryObservation { metadata, metadata_expires_at,
            entries, listing_expires_at } = observation;
        let key = self.key(path);
        let entry_count = entries.len().saturating_add(1);
        let metadata_bytes = path.len()
            .saturating_add(key.capacity().saturating_mul(3))
            .saturating_add(256)
            .saturating_add(meta_bytes(&metadata))
            .saturating_add(entries.iter().fold(0usize, |total, metadata| {
                total.saturating_add(meta_bytes(metadata))
            }));
        // Validity is independent of retention. In particular, an oversized
        // invalid observation must not expire a valid earlier authority.
        let (entry_index, index_bytes) = build_entry_index(&entries, self.case_sensitive)?;
        let byte_count = metadata_bytes
            .saturating_add(index_bytes)
            .saturating_add(preload_records::byte_charge(&key, &entries))
            .saturating_add(std::mem::size_of::<CachedDirectory>());
        let root_key = self.key(&self.root);
        let mut retired = RetiredMetadata::default();
        let mut loads = self.lock_loads()?;
        let mut state = self.lock_state()?;
        if admission.is_some_and(|(slot, revision)| slot.revision() != revision) {
            return Ok(SnapshotPublication::obsolete());
        }
        if ticket.is_some_and(|ticket| ticket.path != path || !preload::ticket_current(&state, ticket)) {
            return Ok(SnapshotPublication::obsolete());
        }
        let previous = state.directories.get(&key).cloned();
        let retainable = byte_count <= MAX_CACHED_DIRECTORY_BYTES;
        let prepared_change = previous.as_ref().filter(|_| retainable).and_then(|previous| {
            state.changes.prepare(
                path,
                changes::SnapshotImage { entries: Arc::clone(&previous.entries),
                    index: Arc::clone(&previous.entry_index), bytes: previous.byte_count },
                changes::SnapshotImage { entries: Arc::clone(&entries),
                    index: Arc::clone(&entry_index), bytes: byte_count },
                self.case_sensitive,
            )
        });
        let mut retained = retainable && (previous.is_none() || prepared_change.is_some());
        let last_access = previous.as_ref().map_or(0, |cached| cached.last_access);
        let additional_bytes = byte_count.saturating_sub(
            previous.as_ref().map_or(0, |cached| cached.byte_count),
        );
        if retained && intent == Admission::Demand {
            evict_until(&mut state, entry_count, additional_bytes, &root_key,
                Some(&key), true, &mut retired);
        }
        retained &= fits(&state, entry_count, additional_bytes);

        // Successful current observation, irrespective of admission: keep the
        // old image solely for comparison. Nothing may serve it as authority,
        // and sibling stats must not repeatedly reload this unretainable parent.
        order::comparison_only(&mut state, &key, Instant::now());
        let completed_revision = publish_observation(&mut loads, &key, admission, &mut retired)?;
        reconcile_parent_authority(&mut state, &mut loads, &key, &metadata, &mut retired);
        reconcile_loads(&mut loads, &key, &entries, &entry_index,
            previous.as_ref().map(|previous| (previous.entries.as_ref(),
                previous.entry_index.as_ref())), self.case_sensitive, retained, &mut retired);
        if let Some(points) = points {
            // Removes stale exact/direct points and identity-replaced descendant
            // points before the snapshot-state lock releases their authority.
            points.reconcile_snapshot(path, &entries,
                previous.as_ref().map(|previous| previous.entries.as_ref()))?;
        }
        reconcile_direct_children(&mut state, &key, &entries, self.case_sensitive,
            previous.as_ref().map(|previous| (previous.entries.as_ref(),
                previous.entry_index.as_ref())), &mut retired);
        state.generation = state.generation.saturating_add(1);
        if !retained {
            if ticket.is_some_and(|ticket| preload::ticket_current(&state, ticket)) {
                self.cool_down_locked(&mut state, &key);
            }
            // The observing caller can still share the complete fresh listing
            // with its existing waiters using this exact new revision.
            return Ok(SnapshotPublication { retained: false, completed_revision });
        }

        let last_touch = tick(&mut state);
        retired.diff(prepared_change.and_then(|prepared| state.changes.commit(prepared)));
        retired.directory(order::detach(&mut state, &key));
        order::insert(&mut state, key.clone(), CachedDirectory {
            path: path.to_string(),
            metadata,
            metadata_expires_at,
            entries: Arc::clone(&entries),
            listing_expires_at,
            entry_index,
            depth,
            entry_count,
            byte_count,
            last_touch,
            last_access,
            refreshed_through_access: last_access,
            revision: last_touch,
            last_attempt: last_touch,
            deferred_changes: false,
            comparison_only: false,
        }, &mut retired);
        order::remove_cooldown(&mut state, &key);
        Ok(SnapshotPublication { retained: true, completed_revision })
        // Reverse declaration order drops state/load guards before retirement,
        // including every early-return error path above.
    }
}
