//! The engine run of the speed cases (U55-4): one transfer job through the
//! transfer lane with the series of its progress messages (rate per second,
//! verdicts, parallelism, ranges) that the cases evaluate.
#![allow(dead_code)]

use smart_explorer::transfer::{
    launch_transfer, Limiter, TransferJob, TransferMsg, TransferRequest,
};
use std::time::{Duration, Instant};

/// What one finished engine job did, with the series the cases evaluate.
pub struct Run {
    pub total: Duration,
    pub first_file: Duration,
    pub files: u64,
    pub bytes: u64,
    pub max_parallel: u32,
    pub max_segments: u32,
    /// The verdict of the final progress (the last one that named a resource).
    pub limiter: Limiter,
    pub link_bps: u64,
    /// (milliseconds since the start, bytes done) per progress message.
    pub samples: Vec<(u64, u64)>,
    /// (milliseconds since the start, verdict) per progress message.
    pub verdicts: Vec<(u64, Limiter)>,
}

impl Run {
    pub fn seconds(&self) -> f64 {
        self.total.as_secs_f64().max(0.001)
    }

    pub fn rate(&self) -> f64 {
        self.bytes as f64 / self.seconds()
    }

    pub fn files_per_second(&self) -> f64 {
        self.files as f64 / self.seconds()
    }

    /// Bytes done at `ms`, from the latest sample at or before it.
    pub fn bytes_at(&self, ms: u64) -> u64 {
        self.samples
            .iter()
            .take_while(|(at, _)| *at <= ms)
            .last()
            .map(|(_, bytes)| *bytes)
            .unwrap_or(0)
    }

    /// Bytes per second between two instants of the run.
    pub fn rate_between(&self, from_ms: u64, to_ms: u64) -> f64 {
        let span = to_ms.saturating_sub(from_ms).max(1) as f64 / 1000.0;
        self.bytes_at(to_ms).saturating_sub(self.bytes_at(from_ms)) as f64 / span
    }

    /// The most frequent verdict among the progress messages of the last
    /// `window_ms`, with its count and the number of messages in the window.
    pub fn tail_verdict(&self, window_ms: u64) -> (Limiter, usize, usize) {
        let end = self.total.as_millis() as u64;
        let from = end.saturating_sub(window_ms);
        let window: Vec<Limiter> = self
            .verdicts
            .iter()
            .filter(|(at, _)| *at >= from)
            .map(|(_, verdict)| *verdict)
            .collect();
        let mut best = (Limiter::None, 0usize);
        for candidate in &window {
            let count = window.iter().filter(|other| *other == candidate).count();
            if count > best.1 {
                best = (*candidate, count);
            }
        }
        (best.0, best.1, window.len())
    }
}

/// How long a canceled job may take to wind down.
const WIND_DOWN: Duration = Duration::from_secs(120);

/// Runs `job` through the transfer lane (as the app does) and waits at most
/// `limit`. Any issue, cancel or timeout panics; a job that times out is
/// canceled and wound down first so it cannot slow the next case.
pub fn run_job(job: TransferJob, limit: Duration) -> Run {
    let started = Instant::now();
    let mut active =
        launch_transfer(TransferRequest::Job(Box::new(job))).expect("the transfer starts");
    let mut run = Run {
        total: Duration::ZERO,
        first_file: Duration::ZERO,
        files: 0,
        bytes: 0,
        max_parallel: 0,
        max_segments: 0,
        limiter: Limiter::None,
        link_bps: 0,
        samples: Vec::new(),
        verdicts: Vec::new(),
    };
    let mut first_file = None;
    let mut last = String::from("no progress yet");
    loop {
        let left = limit.saturating_sub(started.elapsed());
        match active.rx.recv_timeout(left) {
            Ok(TransferMsg::Progress(progress)) => {
                let at = started.elapsed().as_millis() as u64;
                if first_file.is_none() && progress.files_done > 0 {
                    first_file = Some(started.elapsed());
                }
                run.max_parallel = run.max_parallel.max(progress.parallel);
                run.max_segments = run.max_segments.max(progress.segments);
                run.link_bps = run.link_bps.max(progress.link_bps);
                run.samples.push((at, progress.bytes_done));
                run.verdicts.push((at, progress.limiter));
                last = format!(
                    "{}/{} files, {}/{} bytes, {} in parallel, errors {}, limiter {}",
                    progress.files_done,
                    progress.files_total,
                    progress.bytes_done,
                    progress.bytes_total,
                    progress.parallel,
                    progress.errors,
                    progress.limiter.code()
                );
            }
            Ok(TransferMsg::Done {
                progress,
                errors,
                canceled,
                ..
            }) => {
                run.total = started.elapsed();
                if let Some(worker) = active.worker.take() {
                    let _ = worker.join();
                }
                assert!(!canceled, "the transfer was canceled");
                assert!(
                    errors.is_empty(),
                    "the transfer reported errors: {errors:?}"
                );
                run.first_file = first_file.unwrap_or(run.total);
                run.files = progress.files_done;
                run.bytes = progress.bytes_done;
                run.max_parallel = run.max_parallel.max(progress.parallel);
                run.max_segments = run.max_segments.max(progress.segments);
                run.limiter = progress.limiter;
                run.link_bps = run.link_bps.max(progress.link_bps);
                return run;
            }
            Err(_) => {
                active.request_cancel();
                let deadline = Instant::now() + WIND_DOWN;
                let mut wound_down = false;
                while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                    match active.rx.recv_timeout(left) {
                        Ok(TransferMsg::Done { .. }) => {
                            wound_down = true;
                            break;
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
                if wound_down {
                    if let Some(worker) = active.worker.take() {
                        let _ = worker.join();
                    }
                }
                panic!(
                    "the transfer did not finish within {limit:?} (last progress: {last}; \
                     wound down after the cancel: {wound_down})"
                );
            }
        }
    }
}
