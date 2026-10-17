//! Signals of the Linux and Android cache lease (DRV-M2): one host per
//! drive cache, and no lease through a link anywhere on the cache path.
use super::{audit_recovery, CacheLease};
use crate::mount::os::linux_os::cache_space::CacheDiskSpace;
use crate::mount::{CacheSpaceProbe, MountId, MountRecovery};
use std::fs;
use std::io;
use std::os::unix::fs::symlink;

#[test]
fn drive_task_unix_lease_excludes_second_host() {
    let directory = tempfile::tempdir().unwrap();
    let cache = directory.path();
    let id = MountId::parse("unix-lease").unwrap();
    // A drive without a cache directory has nothing to recover.
    assert_eq!(audit_recovery(cache, &id).unwrap(), MountRecovery::Clean);
    let first = CacheLease::acquire(cache, &id).unwrap();
    // A second host of the same drive, even in this process, is refused.
    let error = CacheLease::acquire(cache, &id).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    // Another drive in the same cache is independent.
    let other = CacheLease::acquire(cache, &MountId::parse("other-drive").unwrap()).unwrap();
    assert!(cache.join("unix-lease").join("files").is_dir());
    assert!(cache.join(".unix-lease.host.lock").is_file());
    // The audit of a running drive waits for its host instead of reading.
    let busy = audit_recovery(cache, &id).err().unwrap();
    assert_eq!(busy.kind(), io::ErrorKind::WouldBlock);
    drop(first);
    let second = CacheLease::acquire(cache, &id).unwrap();
    drop(second);
    drop(other);
    assert_eq!(audit_recovery(cache, &id).unwrap(), MountRecovery::Clean);
    // The free-space probe of the same cache answers without a lease.
    let space = CacheDiskSpace::new(cache).unwrap();
    assert!(space.available_bytes().unwrap() > 0);
    assert!(CacheDiskSpace::new(std::path::Path::new("relative")).is_err());
}

#[test]
fn drive_task_unix_lease_rejects_symlinked_cache() {
    let id = MountId::parse("linked").unwrap();
    // The cache root itself is a link.
    let directory = tempfile::tempdir().unwrap();
    let real = directory.path().join("real");
    fs::create_dir(&real).unwrap();
    let linked_root = directory.path().join("linked-root");
    symlink(&real, &linked_root).unwrap();
    assert_refused(CacheLease::acquire(&linked_root, &id));

    // The drive directory is a link.
    let cache = directory.path().join("cache-mount");
    fs::create_dir(&cache).unwrap();
    symlink(&real, cache.join("linked")).unwrap();
    assert_refused(CacheLease::acquire(&cache, &id));

    // The spool directory is a link.
    let cache = directory.path().join("cache-files");
    fs::create_dir_all(cache.join("linked")).unwrap();
    symlink(&real, cache.join("linked").join("files")).unwrap();
    assert_refused(CacheLease::acquire(&cache, &id));

    // The lock file is a link, even to a file of this user.
    let cache = directory.path().join("cache-lock");
    fs::create_dir(&cache).unwrap();
    let target = directory.path().join("target.lock");
    fs::write(&target, b"").unwrap();
    symlink(&target, cache.join(".linked.host.lock")).unwrap();
    assert!(CacheLease::acquire(&cache, &id).is_err());

    // The lock file has a second name.
    let cache = directory.path().join("cache-hardlink");
    fs::create_dir(&cache).unwrap();
    let lock = cache.join(".linked.host.lock");
    fs::write(&lock, b"").unwrap();
    fs::hard_link(&lock, directory.path().join("second-name")).unwrap();
    assert_refused(CacheLease::acquire(&cache, &id));
}

fn assert_refused(result: io::Result<CacheLease>) {
    let error = result.err().expect("a link on the cache path was accepted");
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
}
