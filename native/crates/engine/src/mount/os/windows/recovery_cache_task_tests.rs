use crate::mount::{clean_cache::{CleanCache, IdleClean}, optimization_fixture::FixtureDirectory,
    spool::WholeFileSpool, Baseline, MountId, MountRecovery};
use std::{fs::{self, OpenOptions}, os::windows::fs::OpenOptionsExt, time::Instant};

#[test]
fn mount_recovery_cache_task_windows_locked_clean_file_is_accounted_and_retryable() {
    let dir = FixtureDirectory::new();
    let id = MountId::parse("windows-recovery-cache").unwrap();
    let root = crate::mount::prepare_spool_root(dir.path()).unwrap();
    let _lease = super::cache_lease::CacheLease::acquire(&root, &id).unwrap();
    let (spool, _) = WholeFileSpool::open(&root, &id).unwrap();
    let cache = CleanCache::default();
    let mut names = Vec::new();
    for index in 0..2 {
        let file = spool.allocate().unwrap();
        drop(file.file);
        let path = format!("/note-{index}");
        cache.retain(IdleClean::new(path.clone(), path, file.name.clone(),
            Baseline::Missing, 0, Instant::now())).unwrap();
        names.push(file.name);
    }
    let files = root.join(id.as_str()).join("files");
    let locked = OpenOptions::new().read(true).share_mode(1).open(files.join(&names[0])).unwrap();
    assert!(cache.trim(&spool, 0).is_err());
    assert_eq!(cache.usage().unwrap(), (1, 0));
    assert!(!files.join(&names[1]).exists());
    assert!(files.join(&names[0]).exists());
    drop(locked);
    cache.trim(&spool, 0).unwrap();
    assert_eq!(cache.usage().unwrap(), (0, 0));
    drop(spool);
    drop(_lease);
    assert_eq!(super::cache_lease::audit_recovery(&root, &id).unwrap(), MountRecovery::Clean);
    assert_eq!(fs::read_dir(files).unwrap().count(), 0);
}
