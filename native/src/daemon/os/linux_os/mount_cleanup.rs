//! What a Linux drive host leaves behind, removed by the daemon (DRV-M4,
//! decision C2 "Aufräumen nach Absturz").
//!
//! There is no `auto_unmount` (it needs `allow_other`, and mount.fuse3(8)
//! calls it dangerous): when the daemon observes the end of a host, through
//! `MountManager::tick` or any stop path, a dead se mount still listed at
//! the drive's folder is unmounted lazily (`fusermount3 -u -z`), and the
//! folder is locked again (`0500`). A live mount there belongs to another
//! host and is never touched. The restricted worker of a login restore runs
//! only `MountManager::tick` and cleans up the same way.
use std::path::Path;

use crate::mount::os::linux_os::mount_point;

/// After the host of the drive at `folder` ended.
pub(in crate::daemon) fn clean_after_host(folder: &Path) {
    if mount_point::dead_mount(folder) {
        let target = mount_point::canonical(folder).unwrap_or_else(|_| folder.to_path_buf());
        if let Err(error) = mount_point::unmount(&target, true) {
            super::super::state::log(&format!(
                "Laufwerk-Einhängeort {} nach dem Ende des Hosts aushängen: {error}",
                folder.display()
            ));
        }
    }
    let _ = mount_point::lock(folder, mount_point::place_kind(folder));
}
