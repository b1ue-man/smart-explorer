//! R-FS-4: what one device keeps of retained analysis results. Its kept
//! tasks are at most `RETAINED_PER_PRINCIPAL`, its finished results hold at
//! most half the disk budget of all retained results; other devices keep
//! their share.
use super::{Key, Task};
use crate::share::session::PeerPrincipal;
use std::{collections::HashMap, io, sync::Arc, time::Instant};

/// Retained tasks one device keeps at most.
const RETAINED_PER_PRINCIPAL: usize = 4;

/// R-FS-4: before `principal` keeps another task, its oldest finished
/// result nobody is attached to goes while it is over its share; a device
/// whose kept tasks all run or are attached is told to retry later. Other
/// devices' results and every attached or running task stay.
pub(super) fn make_room(
    table: &mut HashMap<Key, Arc<Task>>,
    principal: &PeerPrincipal,
) -> io::Result<()> {
    let share = crate::share::analysis_resources::disk_limit() / 2;
    loop {
        let own: Vec<(&Key, &Arc<Task>)> = table
            .iter()
            .filter(|(key, _)| &key.0 == principal)
            .collect();
        let bytes: u64 = own.iter().map(|(_, task)| task.retained_bytes()).sum();
        if own.len() < RETAINED_PER_PRINCIPAL && bytes <= share {
            return Ok(());
        }
        let oldest = own
            .iter()
            .filter_map(|(key, task)| task.detached_result_since().map(|at| (at, *key)))
            .min_by_key(|(at, _)| *at)
            .map(|(_, key)| key.clone());
        let Some(task) = oldest.and_then(|key| table.remove(&key)) else {
            return Err(crate::vfs::congestion_error(
                "Zu viele vorgehaltene Analyseaufträge dieses Geräts",
                None,
            ));
        };
        // The grace timer still holds the task: drop the spool now.
        task.finish(Err(io::Error::other("Analyseergebnis verworfen")));
    }
}

impl Task {
    /// The bytes of the finished result this task keeps on disk.
    fn retained_bytes(&self) -> u64 {
        match &*self.result.lock().unwrap_or_else(|p| p.into_inner()) {
            Some(Ok(spool)) => spool.bytes,
            _ => 0,
        }
    }
    /// When the last attachment ended, for a finished task nobody is
    /// attached to; `None` while it runs or is attached.
    fn detached_result_since(&self) -> Option<Instant> {
        let finished = self
            .result
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some();
        let attached = self.attached.lock().unwrap_or_else(|p| p.into_inner());
        (finished && attached.0 == 0).then_some(attached.1)
    }
}

#[cfg(test)]
#[path = "analysis_tasks_retention_task_tests.rs"]
mod task_tests;
