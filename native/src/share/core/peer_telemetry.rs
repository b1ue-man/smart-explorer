use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;

use super::types::ShareEvent;
use super::wire::FsResponse;

const SAMPLE_EVERY: u64 = 128;
const ALWAYS_REPORT_AFTER: Duration = Duration::from_millis(500);
static SUCCESS_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn report_fs_success(
    events: &Sender<ShareEvent>,
    operation: &str,
    started: Instant,
    response: &FsResponse,
) {
    let sequence = SUCCESS_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let elapsed = started.elapsed();
    if elapsed < ALWAYS_REPORT_AFTER && sequence % SAMPLE_EVERY != 0 {
        return;
    }
    try_emit(
        events,
        ShareEvent::Status(format!(
            "Share-Op {operation}: {} ms, {}",
            elapsed.as_millis(),
            super::peer_fs_logging::response_summary(response)
        )),
    );
}

pub(super) fn report_exec_success(events: &Sender<ShareEvent>, message: String) {
    try_emit(events, ShareEvent::Status(message));
}

fn try_emit(events: &Sender<ShareEvent>, event: ShareEvent) {
    let _ = events.try_send(event);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_drive_task_full_telemetry_channel_never_blocks_peer_operation() {
        let (events, _receiver) = crossbeam_channel::bounded(1);
        events
            .send(ShareEvent::Status("already full".into()))
            .unwrap();
        try_emit(&events, ShareEvent::Status("must be dropped".into()));
        assert_eq!(events.len(), 1);
    }
}
