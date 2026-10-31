//! Stages written with the source's time over the agent protocol
//! (`stage-mtime-v1`, W5): the SSH agent sets the time on the open stage
//! before it closes it and answers the commit with what that handle shows
//! (`StageWritten`: size, time, file identity). That answer is the stage's
//! host evidence (`take_stage_evidence`), so publishing sends neither a
//! `FinishStage` (`transfer/.../publish_time.rs` sees the time in it) nor a
//! `Stat` and `FileIdentity` for its recovery record. A server without the
//! label (an older agent, the background service) gets the plain stage and
//! `FinishStage` afterwards, as before.
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};

use super::backend::AgentBackend;
use crate::agent_proto::Frame;
use crate::vfs::{Backend, FileEvidence, StageEvidence, VfsResult};

/// Reports of committed timed stages, until their publication takes them.
#[derive(Clone, Default)]
pub(super) struct StageReports(Arc<Mutex<HashMap<String, StageEvidence>>>);

impl StageReports {
    fn put(&self, evidence: StageEvidence) {
        let mut reports = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reports.insert(evidence.stage.path.clone(), evidence);
    }

    /// `vfs::take_stage_evidence`: the report of `stage`, once.
    pub(super) fn take(&self, stage: &str) -> Option<StageEvidence> {
        let mut reports = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reports.remove(stage)
    }
}

/// `open_write_copy_stage_timed` of an agent connection.
pub(super) fn open(
    backend: &AgentBackend,
    path: &str,
    size: u64,
    mtime_ms: i64,
) -> VfsResult<Box<dyn Write + Send>> {
    let features = backend.features();
    if mtime_ms <= 0 || features.service || !features.stage_mtime {
        return backend.open_write_copy_stage_sized(path, size);
    }
    let reports = backend.pool.stage_reports();
    let stage = path.to_string();
    let report: super::stream::StageReport = Box::new(move |size, mtime_ms, object| {
        reports.put(StageEvidence {
            stage: FileEvidence {
                path: stage,
                object,
                id: None,
                size,
                mtime_ms,
            },
            destination: None,
            existed: None,
        });
    });
    let request = Frame::WriteNewTimed {
        path: path.to_string(),
        mtime_ms,
    };
    backend.agent_open_write_reporting(request, Some(report))
}
