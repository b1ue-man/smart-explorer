use super::*;
use std::sync::mpsc;
use std::time::Instant;

fn request() -> TransferRequest {
    TransferRequest::Download {
        backend: Arc::new(crate::vfs::LocalBackend::new("/")),
        files: vec!["/never/run".into()],
        dest_local: "/never/run".into(),
        filter: None,
    }
}

/// Workers that finish only when released, so admission is observable.
struct Releases(Vec<mpsc::Sender<bool>>);

impl Releases {
    fn launcher(&mut self) -> impl FnMut(TransferRequest) -> Result<ActiveTransfer, String> + '_ {
        move |request| {
            let (tx, rx) = unbounded();
            let (release_tx, release_rx) = mpsc::channel::<bool>();
            self.0.push(release_tx);
            let progress =
                TransferProgress::new(request.kind(), "test", request.item_count() as u64, 0);
            let done_progress = progress.clone();
            let worker = std::thread::spawn(move || {
                if release_rx.recv().unwrap_or(false) {
                    let _ = tx.send(TransferMsg::Done {
                        progress: done_progress,
                        errors: Vec::new(),
                        canceled: false,
                        issues: Vec::new(),
                        roots: Vec::new(),
                    });
                }
                // `false` ends the worker without a terminal message.
            });
            Ok(ActiveTransfer {
                rx,
                progress,
                cancel: Arc::new(AtomicBool::new(false)),
                worker: Some(worker),
                job: None,
            })
        }
    }
}

fn wait_finished(lane: &mut TransferLane) -> Vec<FinishedTransfer> {
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let finished = lane.poll();
        if !finished.is_empty() || Instant::now() > deadline {
            return finished;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn transfer_engine_task_transfer_lane_starts_every_request_at_once() {
    let mut releases = Releases(Vec::new());
    let mut lane = TransferLane::new();
    {
        let mut launch = releases.launcher();
        for _ in 0..10 {
            assert_eq!(lane.submit(request(), &mut launch), Ok(()));
        }
    }
    assert_eq!(lane.active.len(), 10, "no fixed number of transfers");
    assert!(lane.poll().is_empty(), "nothing finished yet");
    assert!(lane.workers_unfinished());

    releases.0[0].send(true).unwrap();
    let finished = wait_finished(&mut lane);
    assert_eq!(finished.len(), 1);
    assert!(finished[0].outcome.is_some() && !finished[0].cancel_requested);
    assert!(finished[0].job.is_none(), "legacy requests carry no job");
    assert_eq!(lane.active.len(), 9);

    lane.cancel(0);
    assert!(lane.active[0].canceling() && !lane.active[1].canceling());
    for release in &releases.0[1..] {
        let _ = release.send(true);
    }
    let mut done = 0;
    while done < 9 {
        done += wait_finished(&mut lane).len();
    }
    assert!(lane.is_idle());
}

#[test]
fn transfer_engine_task_transfer_lane_reports_lost_workers_and_shuts_down() {
    let mut releases = Releases(Vec::new());
    let mut lane = TransferLane::new();
    {
        let mut launch = releases.launcher();
        assert_eq!(lane.submit(request(), &mut launch), Ok(()));
        assert_eq!(lane.submit(request(), &mut launch), Ok(()));
    }
    // Ending the worker without a terminal message is reported as lost.
    releases.0[0].send(false).unwrap();
    let finished = wait_finished(&mut lane);
    assert_eq!(finished.len(), 1);
    assert!(finished[0].outcome.is_none());
    assert_eq!(lane.active.len(), 1);
    lane.shutdown();
    assert!(lane.is_idle(), "shutdown drops the running entries");
}
