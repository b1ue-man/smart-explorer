//! Per-path sharing aggregates and their handle-lifecycle transitions.
//! All methods run under the owning handle-state mutex and perform no I/O.
//! Admission examines three access categories, never the handles sharing a path.
use std::collections::{HashMap, HashSet};
use std::io;

use super::super::handle_access::{
    invalid_handle, requests_delete, requests_read, requests_write, share_allows, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE,
};
use super::validation::snapshot_record;
use super::{HandleRecord, HandleSnapshot, State};

#[cfg(test)]
#[path = "share_index_task_tests.rs"]
mod task_tests;

const SHARE_BITS: [u32; 3] = [FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_SHARE_DELETE];

fn requests(access: u32) -> [bool; 3] {
    [requests_read(access), requests_write(access), requests_delete(access)]
}

#[derive(Clone, Copy, Default)]
struct Counts {
    // Cleaned-up but not closed handles still identify an attached destination.
    attached: usize,
    requested: [usize; 3],
    denied: [usize; 3],
}

impl Counts {
    fn changed(mut self, record: &HandleRecord, add: bool, attached: bool) -> io::Result<Self> {
        if attached {
            self.attached = adjusted(self.attached, 1, add)?;
        }
        if record.share_active {
            let requested = requests(record.desired_access);
            for index in 0..SHARE_BITS.len() {
                self.requested[index] = adjusted(
                    self.requested[index], usize::from(requested[index]), add,
                )?;
                self.denied[index] = adjusted(
                    self.denied[index],
                    usize::from(record.share_access & SHARE_BITS[index] == 0), add,
                )?;
            }
        }
        Ok(self)
    }

    fn allows(self, desired_access: u32, share_access: u32) -> bool {
        let shared_by_every_handle = SHARE_BITS.iter().enumerate().fold(0, |mask, (index, bit)| {
            mask | if self.denied[index] == 0 { *bit } else { 0 }
        });
        share_allows(shared_by_every_handle, desired_access)
            && (0..SHARE_BITS.len()).all(|index| {
                share_access & SHARE_BITS[index] != 0 || self.requested[index] == 0
            })
    }
}

fn adjusted(value: usize, delta: usize, add: bool) -> io::Result<usize> {
    let updated = if add { value.checked_add(delta) } else { value.checked_sub(delta) };
    updated.ok_or_else(|| io::Error::other("file-handle sharing counters are inconsistent"))
}

#[derive(Default)]
pub(super) struct ShareIndex {
    paths: HashMap<String, Counts>,
}

impl ShareIndex {
    pub(super) fn allows(&self, path: &str, desired_access: u32, share_access: u32) -> bool {
        self.paths.get(path).map_or(true, |counts| counts.allows(desired_access, share_access))
    }

    pub(super) fn has_attached(&self, path: &str) -> bool {
        self.paths.contains_key(path)
    }

    pub(super) fn delete_allowed_except(&self, path: &str, except: Option<&HandleRecord>) -> bool {
        let excluded = usize::from(except.is_some_and(|record| {
            record.namespace_attached && record.share_active && record.path == path
                && record.share_access & FILE_SHARE_DELETE == 0
        }));
        self.paths.get(path).map_or(0, |counts| counts.denied[2]) <= excluded
    }

    fn add(&mut self, record: &HandleRecord) -> io::Result<()> {
        if !record.namespace_attached {
            return Ok(());
        }
        let counts = self.paths.get(&record.path).copied().unwrap_or_default();
        let updated = counts.changed(record, true, true)?;
        self.paths.insert(record.path.clone(), updated);
        Ok(())
    }

    fn remove(&mut self, record: &HandleRecord) -> io::Result<()> {
        if !record.namespace_attached {
            return Ok(());
        }
        let counts = self.paths.get_mut(&record.path).ok_or_else(|| {
            io::Error::other("attached handle has no sharing counters")
        })?;
        let updated = counts.changed(record, false, true)?;
        if updated.attached == 0 {
            self.paths.remove(&record.path);
        } else {
            *counts = updated;
        }
        Ok(())
    }

    fn deactivate(&mut self, record: &HandleRecord) -> io::Result<()> {
        if record.namespace_attached && record.share_active {
            let counts = self.paths.get_mut(&record.path).ok_or_else(|| {
                io::Error::other("active handle has no sharing counters")
            })?;
            *counts = counts.changed(record, false, false)?;
        }
        Ok(())
    }
}

impl State {
    pub(super) fn insert_handle(&mut self, key: u64, record: HandleRecord) -> io::Result<()> {
        if self.handles.contains_key(&key) {
            return Err(invalid_handle("file handle identifier is already reserved"));
        }
        // Unbound reservations participate exactly like fully opened handles.
        self.shares.add(&record)?;
        self.handles.insert(key, record);
        Ok(())
    }

    pub(super) fn cleanup_handle(&mut self, key: u64) -> io::Result<HandleSnapshot> {
        let record = self.handles.get_mut(&key)
            .ok_or_else(|| invalid_handle("unknown file handle"))?;
        self.shares.deactivate(record)?;
        record.share_active = false;
        snapshot_record(record)
    }

    pub(super) fn remove_handle(&mut self, key: u64) -> io::Result<HandleRecord> {
        let record = self.handles.get(&key)
            .ok_or_else(|| invalid_handle("unknown file handle"))?;
        self.shares.remove(record)?;
        self.handles.remove(&key).ok_or_else(|| invalid_handle("unknown file handle"))
    }

    pub(super) fn complete_delete(&mut self, path: &str, requesters: &HashSet<u64>) {
        self.shares.paths.remove(path);
        for (key, record) in &mut self.handles {
            if record.namespace_attached && record.path == path {
                record.namespace_attached = false;
            }
            if requesters.contains(key) {
                record.delete_requested = false;
                record.delete_committed = true;
            }
        }
    }

    pub(super) fn rename_attached(
        &mut self, source: &str, destination: &str, replace_existing: bool,
    ) -> io::Result<()> {
        // Reservation paths are canonical. A successful same-object no-op
        // leaves all attached handles and their aggregate contributions intact.
        if source == destination {
            return Ok(());
        }
        if replace_existing {
            self.shares.paths.remove(destination);
            for record in self.handles.values_mut()
                .filter(|record| record.namespace_attached && record.path == destination)
            {
                record.namespace_attached = false;
            }
        }
        let prefix = format!("{}\\", source.trim_end_matches('\\'));
        for record in self.handles.values_mut() {
            if !record.namespace_attached {
                continue;
            }
            let updated = if record.path == source {
                destination.to_string()
            } else if let Some(suffix) = record.path.strip_prefix(&prefix) {
                format!("{}\\{suffix}", destination.trim_end_matches('\\'))
            } else {
                continue;
            };
            self.shares.remove(record)?;
            record.path = updated;
            self.shares.add(record)?;
        }
        Ok(())
    }
}
