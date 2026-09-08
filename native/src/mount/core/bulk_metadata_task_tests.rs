//! Publication fences for the single remote mount_vault_task suite.
use super::{Admission, DirectoryObservation, MetadataCache, MetadataLookup, MAX_CACHED_BYTES};
use super::vault_task_tests::{directory, file, observation, observation_at};
use crate::mount::metadata_point_cache::MetadataPointCache;
use crate::vfs::VfsMeta;
use std::{io, sync::Arc, time::Instant};

#[derive(Clone, Copy, Debug)]
enum Rejection { Notification, Capacity, Oversized }

fn image(entries: &Arc<[VfsMeta]>) -> DirectoryObservation {
    let mut image = observation(Vec::new());
    image.entries = Arc::clone(entries);
    image
}

#[test]
fn mount_vault_task_unretained_observation_fences_authority_and_completes_exact_revision() -> io::Result<()> {
    for rejection in [Rejection::Notification, Rejection::Capacity, Rejection::Oversized] {
        let cache = MetadataCache::new("/", true);
        let points = MetadataPointCache::new(true);
        let root = cache.load_slot("/")?;
        assert!(cache.install_observation("/", observation(vec![file("note", 1), directory("space")]),
            0, Some((&root, root.revision())), Admission::Demand)?);
        assert!(cache.install_observation("/space", observation_at("/space", Vec::new()),
            1, None, Admission::Demand)?);
        let baseline = cache.directory("/")?.unwrap();
        let snapshot_revision = cache.revision("/")?;
        let revision = root.revision();
        let note = cache.load_slot("/note")?;
        let absent = cache.load_slot("/space0")?;
        let deep = cache.load_slot("/space/deep")?;
        let (note_revision, absent_revision, deep_revision) =
            (note.revision(), absent.revision(), deep.revision());
        points.install("/note", file("note", 0))?;
        root.complete_directory(revision, Instant::now() + super::DIRECTORY_TTL,
            Arc::clone(&baseline))?;
        let mut next = vec![file("note", 2), directory("space"), file("extra", 2)];
        match rejection {
            Rejection::Notification => { cache.test_change_budget(Some(0))?; }
            Rejection::Capacity => cache.test_fill_retention("/space")?,
            Rejection::Oversized => {
                // Charge actual allocation capacity without filling 128 MiB or
                // cloning the String (which would discard its spare capacity).
                let mut identity = String::new();
                identity.try_reserve_exact(MAX_CACHED_BYTES + 1)
                    .map_err(|error| io::Error::other(error.to_string()))?;
                identity.push('x');
                next[0].id = Some(identity);
            }
        }
        let next: Arc<[VfsMeta]> = next.into();
        let obsolete = cache.install_observation_publication("/", image(&next), 0,
            &root, revision.wrapping_sub(1), Admission::Refresh, &points)?;
        assert!(!obsolete.retained && obsolete.completed_revision.is_none());
        assert!(Arc::ptr_eq(&baseline, &cache.directory("/")?.unwrap()));
        assert_eq!(root.revision(), revision, "obsolete {rejection:?} observation changes nothing");
        assert_eq!(note.revision(), note_revision);

        let publication = cache.install_observation_publication("/", image(&next), 0,
            &root, revision, Admission::Refresh, &points)?;
        assert!(!publication.retained, "fixture must exercise {rejection:?} rejection");
        let completed = revision.wrapping_add(1);
        assert_eq!(publication.completed_revision, Some(completed));
        assert_eq!(root.revision(), completed);
        assert_eq!(cache.revision("/")?, snapshot_revision, "old image remains comparison-owned");
        assert!(root.completed_directory()?.is_none(), "old shared authority was retired");
        assert_ne!(note.revision(), note_revision);
        assert_ne!(absent.revision(), absent_revision, "fence points absent from both images");
        assert_eq!(deep.revision(), deep_revision, "unchanged deep branch is not canceled");
        assert!(cache.directory("/")?.is_none());
        for path in ["/", "/note", "/missing"] {
            assert!(matches!(cache.stat(path)?, MetadataLookup::Uncached));
            assert!(cache.metadata_hint(path)?.is_none());
        }
        assert!(matches!(points.lookup("/note")?, MetadataLookup::Uncached));
        assert!(cache.expired_parent("/note")?.is_none());
        {
            let state = cache.lock_state()?;
            let old = state.directories.get("/").unwrap();
            assert!(old.comparison_only && Arc::ptr_eq(&old.entries, &baseline));
        }
        assert!(!cache.install_point_if_current("/space0", &absent, absent_revision,
            &points, Some(file("space0", 0)))?, "a pre-observation point cannot publish");
        root.complete_directory(revision, Instant::now() + super::DIRECTORY_TTL,
            Arc::clone(&baseline))?;
        assert!(root.completed_directory()?.is_none());
        root.complete_directory(completed, Instant::now() + super::DIRECTORY_TTL, Arc::clone(&next))?;
        assert!(Arc::ptr_eq(&next, &root.completed_directory()?.unwrap()),
            "unretained fresh data is shareable only at its exact completion revision");
        root.invalidate();
        root.complete_directory(completed, Instant::now() + super::DIRECTORY_TTL, Arc::clone(&next))?;
        assert!(root.completed_directory()?.is_none(), "a later invalidation cannot borrow completion");
    }
    Ok(())
}

#[test]
fn mount_vault_task_unchanged_parent_preserves_child_flight_and_atomically_refuses_stale_points() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    let points = MetadataPointCache::new(true);
    let entries = vec![directory("a"), file("note", 1)];
    assert!(cache.install_observation("/", observation(entries.clone()), 0, None, Admission::Demand)?);
    let child = cache.load_slot("/a")?;
    let point = cache.load_slot("/note")?;
    let absent = cache.load_slot("/missing")?;
    let (child_revision, point_revision, absent_revision) =
        (child.revision(), point.revision(), absent.revision());
    cache.test_expire_directory("/")?;
    assert!(matches!(cache.stat("/note")?, MetadataLookup::Uncached));
    assert!(matches!(cache.stat("/missing")?, MetadataLookup::Uncached));
    // These are the point caller's last external rechecks. Parent publication
    // now interleaves before install_point_if_current acquires load -> state.
    assert!(cache.install_observation_reconciled("/", observation(entries.clone()),
        0, None, Admission::Refresh, &points)?);
    assert_eq!(child.revision(), child_revision, "unchanged directory fetch stays admissible");
    assert_eq!(point.revision(), point_revision);
    assert_eq!(absent.revision(), absent_revision);
    assert!(!cache.install_point_if_current("/note", &point, point_revision,
        &points, Some(file("note", 999)))?);
    assert!(!cache.install_point_if_current("/missing", &absent, absent_revision,
        &points, Some(file("missing", 999)))?);
    assert!(matches!(points.lookup("/note")?, MetadataLookup::Uncached));
    assert!(matches!(points.lookup("/missing")?, MetadataLookup::Uncached));
    let MetadataLookup::Found(note) = cache.stat("/note")? else { panic!("snapshot lost authority"); };
    assert_eq!(note.mtime_ms, 1);
    assert!(matches!(cache.stat("/missing")?, MetadataLookup::KnownMissing));
    assert!(cache.install_observation("/a", observation_at("/a", vec![file("child", 1)]),
        1, Some((&child, child_revision)), Admission::Demand)?);

    let revision = child.revision();
    let mut changed = entries;
    changed[0].id = Some("replacement-a".into());
    assert!(cache.install_observation_reconciled("/", observation(changed),
        0, None, Admission::Refresh, &points)?);
    assert_ne!(child.revision(), revision, "changed exact child still fences its old flight");
    assert!(!cache.install_observation("/a", observation_at("/a", vec![file("stale", 1)]),
        1, Some((&child, revision)), Admission::Demand)?);
    Ok(())
}
