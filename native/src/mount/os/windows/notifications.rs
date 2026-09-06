//! DLL notifications are issued only by the filesystem-owning host loop.
use std::{collections::VecDeque, io};

use crate::mount::{metadata_cache::MetadataChange, DriveLetter, MountEngine};
use super::DokanyFileSystem;

const NOTIFICATION_BATCH: usize = 128;

#[derive(Default)]
pub(super) struct HostNotifications {
    pending: VecDeque<MetadataChange>,
}

impl HostNotifications {
    pub(super) fn deliver(
        &mut self,
        engine: &MountEngine,
        filesystem: &DokanyFileSystem,
        drive: DriveLetter,
    ) -> io::Result<()> {
        if self.pending.is_empty() {
            for change in engine.drain_metadata_changes(NOTIFICATION_BATCH)? {
                let recursive = matches!(&change,
                    MetadataChange::Deleted { is_directory: true, .. }
                    | MetadataChange::Modified { .. });
                engine.invalidate_content(change.path(), recursive);
                self.pending.push_back(change);
            }
        }
        for _ in 0..NOTIFICATION_BATCH {
            let Some(change) = self.pending.front() else { break };
            let path = mounted_path(engine, drive, change.path())?;
            let delivered = match change {
                MetadataChange::Created { is_directory, .. } => {
                    filesystem.notify_create(&path, *is_directory)
                }
                MetadataChange::Deleted { is_directory, .. } => {
                    filesystem.notify_delete(&path, *is_directory)
                }
                MetadataChange::Modified { .. } => filesystem.notify_update(&path),
            };
            if !delivered {
                // Retain this bounded batch; a transient delivery failure must
                // not turn an already drained change into a lost notification.
                return Err(io::Error::other("Dokany remote-change notification was not accepted"));
            }
            self.pending.pop_front();
        }
        Ok(())
    }
}

fn mounted_path(engine: &MountEngine, drive: DriveLetter, backend: &str) -> io::Result<Vec<u16>> {
    let root = engine.projector.root().as_str();
    let suffix = backend.strip_prefix(root)
        .filter(|suffix| root == "/" || suffix.is_empty() || suffix.starts_with('/'))
        .ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied,
            "notification path escaped its mounted root"))?;
    let callback = format!("\\{}", suffix.trim_start_matches('/').replace('/', "\\"));
    if engine.projector.project(&callback)?.backend() != backend {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "notification path is not canonical"));
    }
    let mut path = format!("{}:{callback}", drive.get()).encode_utf16().collect::<Vec<_>>();
    if path.len() >= 32_767 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "notification pathname is too long"));
    }
    path.push(0);
    Ok(path)
}
