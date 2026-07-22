use super::metadata_cache::MetadataCache;
use crate::vfs::VfsMeta;
use std::io;
use std::sync::Arc;

#[test]
fn remote_drive_task_cached_enumerations_share_one_immutable_snapshot() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    let entries: Arc<[VfsMeta]> = vec![file("note.md", 12)].into();
    assert!(cache.install_directory("/", directory("/"), Arc::clone(&entries), 0)?);

    let first = cache.directory("/")?.expect("root snapshot");
    let second = cache.directory("/")?.expect("root snapshot");
    assert!(Arc::ptr_eq(&entries, &first));
    assert!(Arc::ptr_eq(&first, &second));
    Ok(())
}

#[test]
fn remote_drive_task_snapshot_generation_detects_parent_and_point_observations() -> io::Result<()> {
    let cache = MetadataCache::new("/", true);
    let initial = cache.generation()?;
    assert!(cache.install_directory("/", directory("/"), Vec::new().into(), 0)?);
    let parent = cache.generation()?;
    assert_ne!(parent, initial);
    cache.note_external_observation()?;
    assert_ne!(cache.generation()?, parent);
    Ok(())
}

fn directory(name: &str) -> VfsMeta {
    VfsMeta {
        name: name.into(),
        is_dir: true,
        ..VfsMeta::default()
    }
}

fn file(name: &str, size: u64) -> VfsMeta {
    VfsMeta {
        name: name.into(),
        size,
        ..VfsMeta::default()
    }
}
