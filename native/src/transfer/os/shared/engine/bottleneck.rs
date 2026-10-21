//! Who limits this job: once a second the time accounts of the last window,
//! the state of the job's flows and the system sensors give a verdict
//! (`core/limiter.rs`), which is shown only after three equal windows. The
//! progress value carries the verdict, its German text, where the time went
//! and the link rate. The final value carries the last verdict that named a
//! resource, so `--stats` and the finished list tell what limited the job
//! even though the queue ran empty at the very end.
//!
//! Texts name drives, devices and kinds of endpoints only, never a file or a
//! folder: the MCP and the CLI pass them on.
use super::super::flow::FlowSnapshot;
use super::super::limiter::{
    judge, resolve_local_pair, shares, text, Limiter, LocalPair, Shares, Steady, TextContext,
    WindowFacts,
};
use super::super::sensors::{self, SystemSample};
use super::super::types::TransferProgress;
use super::stats::Totals;
use super::view::Side;
use super::{first_source, lock, Engine};
use crate::format::format_bytes;
use crate::vfs::Scheme;
use std::time::{Duration, Instant};

/// A verdict is formed over about this long.
const WINDOW: Duration = Duration::from_secs(1);

/// The state of the verdict of one job.
pub(super) struct Judgement {
    steady: Steady,
    window_start: Option<Instant>,
    window_totals: Totals,
    shown: Shown,
    /// The last confirmed verdict that named a resource.
    meaningful: Shown,
    shares: Shares,
    link_bps: u64,
    names: Option<Names>,
}

impl Judgement {
    pub(super) fn new() -> Self {
        Self {
            steady: Steady::default(),
            window_start: None,
            window_totals: Totals::default(),
            shown: Shown::default(),
            meaningful: Shown::default(),
            shares: Shares::default(),
            link_bps: 0,
            names: None,
        }
    }
}

/// A confirmed verdict with its text.
#[derive(Clone, Debug, Default)]
struct Shown {
    limiter: Limiter,
    text: Option<String>,
}

/// Names of the sides, found once.
struct Names {
    source_drive: String,
    target_drive: String,
    same_volume: bool,
    source_path: String,
    target_path: String,
}

/// What kind of endpoint a backend is, for the texts: never an address, an
/// account or a path.
pub(super) fn endpoint_name(side: Side<'_>) -> String {
    let Side::Remote(backend) = side else {
        return String::new();
    };
    let name = match backend.scheme() {
        Scheme::Sftp => "SFTP-Server",
        Scheme::Ftp => "FTP-Server",
        Scheme::Webdav => "WebDAV-Server",
        Scheme::Smb => "SMB-Freigabe",
        Scheme::GDrive => "Google Drive",
        Scheme::Peer => "Share-Gerät",
        Scheme::Vault => "Tresor",
        Scheme::Local => "",
    };
    name.to_string()
}

/// The text context of a verdict and the verdict as the sides name it: a
/// local-to-local copy names the drive with the longer queue (or both).
pub(super) fn describe(
    verdict: Limiter,
    names: &NamesView<'_>,
    rate_bps: u64,
    link_bps: u64,
) -> (Limiter, Option<String>) {
    let mut context = TextContext {
        source_name: names.source_drive.to_string(),
        target_name: names.target_drive.to_string(),
        remote_name: names.remote.to_string(),
        rate: format!("{}/s", format_bytes(rate_bps)),
        link: format!("{}/s", format_bytes(link_bps)),
    };
    let mut verdict = verdict;
    if let Some(queues) = names.local_pair {
        let pair = LocalPair {
            source: names.source_drive,
            target: names.target_drive,
            same_volume: names.same_volume,
            source_queue: queues.0,
            target_queue: queues.1,
        };
        let (resolved, drive) = resolve_local_pair(verdict, &pair);
        verdict = resolved;
        match verdict {
            Limiter::Source => context.source_name = drive,
            Limiter::Target => context.target_name = drive,
            _ => {}
        }
    }
    (verdict, text(verdict, &context))
}

/// The inputs of `describe`.
pub(super) struct NamesView<'a> {
    pub source_drive: &'a str,
    pub target_drive: &'a str,
    pub remote: &'a str,
    pub same_volume: bool,
    /// Queue depths of the two drives of a local-to-local copy.
    pub local_pair: Option<(Option<f64>, Option<f64>)>,
}

impl Engine<'_> {
    fn names(&self, judgement: &mut Judgement) {
        if judgement.names.is_some() {
            return;
        }
        let source_path = first_source(self.view.items).to_string();
        let target_path = self.folders.root().to_string();
        let drive = |side: Side<'_>, path: &str| match side {
            Side::Local => sensors::volume_name(path).unwrap_or_default(),
            Side::Remote(_) => String::new(),
        };
        let both_local = self.view.source.is_local() && self.view.target.is_local();
        judgement.names = Some(Names {
            source_drive: drive(self.view.source, &source_path),
            target_drive: drive(self.view.target, &target_path),
            same_volume: both_local && sensors::same_volume(&source_path, &target_path),
            source_path,
            target_path,
        });
    }

    /// The flow whose permits were waited for longer: the one that holds the
    /// job back.
    fn limiting_flow(&self) -> (FlowSnapshot, bool) {
        let source = self.op_flows.0.snapshot();
        match &self.op_flows.1 {
            Some(flow) => {
                let target = flow.snapshot();
                if target.waited_ms >= source.waited_ms {
                    (target, true)
                } else {
                    (source, false)
                }
            }
            None => (source, false),
        }
    }

    /// The remote endpoint a `Remote` verdict is about.
    fn limiting_remote(&self, target_limits: bool) -> String {
        match (self.view.source, self.view.target) {
            (Side::Local, target @ Side::Remote(_)) => endpoint_name(target),
            (source @ Side::Remote(_), Side::Local) => endpoint_name(source),
            (source, target) if target_limits => {
                let name = endpoint_name(target);
                if name.is_empty() {
                    endpoint_name(source)
                } else {
                    name
                }
            }
            (source, target) => {
                let name = endpoint_name(source);
                if name.is_empty() {
                    endpoint_name(target)
                } else {
                    name
                }
            }
        }
    }

    /// Forms the verdict of the window that just ended.
    fn close_window(&self, judgement: &mut Judgement, span: Duration, delta: Totals) {
        self.names(judgement);
        let sample: SystemSample = sensors::latest();
        let (flow, target_limits) = self.limiting_flow();
        let span_ms = span.as_millis().max(1) as u64;
        let both_local = self.view.source.is_local() && self.view.target.is_local();
        let goodput_bps = (u128::from(delta.moved) * 1000 / u128::from(span_ms)) as u64;
        let facts = WindowFacts {
            span_ms,
            saturated: flow.saturated,
            last_up_probe_gained: !flow.plateau,
            source_ns: delta.accounts.source_ns,
            target_ns: delta.accounts.target_ns,
            cpu_ns: delta.accounts.cpu_ns,
            wait_ns: delta.accounts.wait_ns,
            memory_wait_ns: delta.memory_wait_ns,
            source_local: self.view.source.is_local(),
            target_local: self.view.target.is_local(),
            goodput_bps,
            link_bps: if both_local { None } else { sample.link_bps },
            process_cpu_cores: sample.process_cpu_known.then_some(sample.process_cpu_cores),
            cores_available: Some(sample.cores_available),
        };
        // A window without any measured time keeps the last split.
        let split = shares(&facts);
        if split != Shares::default() {
            judgement.shares = split;
        }
        judgement.link_bps = facts.link_bps.unwrap_or(0);
        let confirmed = judgement.steady.push(judge(&facts));
        let Some(names) = judgement.names.as_ref() else {
            return;
        };
        let remote = self.limiting_remote(target_limits);
        let queues = both_local.then(|| {
            (
                sensors::disk_queue_for(&names.source_path),
                sensors::disk_queue_for(&names.target_path),
            )
        });
        let view = NamesView {
            source_drive: &names.source_drive,
            target_drive: &names.target_drive,
            remote: &remote,
            same_volume: names.same_volume,
            local_pair: queues,
        };
        let (limiter, text) = describe(confirmed, &view, goodput_bps, facts.link_bps.unwrap_or(0));
        judgement.shown = Shown { limiter, text };
        if limiter.names_a_resource() {
            judgement.meaningful = judgement.shown.clone();
        }
    }

    /// Puts the verdict into `progress`; a window of about a second that
    /// ended since the last call is judged first. `last` asks for the last
    /// verdict that named a resource instead of the current one.
    pub(super) fn judge_into(&self, progress: &mut TransferProgress, last: bool) {
        let mut guard = lock(&self.judgement);
        let judgement: &mut Judgement = &mut guard;
        let now = Instant::now();
        let totals = self.stats.totals();
        let window_start = judgement.window_start;
        match window_start {
            None => {
                judgement.window_start = Some(now);
                judgement.window_totals = totals;
            }
            Some(started) if now.duration_since(started) >= WINDOW => {
                let delta = totals.since(judgement.window_totals);
                judgement.window_start = Some(now);
                judgement.window_totals = totals;
                self.close_window(judgement, now.duration_since(started), delta);
            }
            Some(_) => {}
        }
        let shown = if last && judgement.meaningful.limiter.names_a_resource() {
            &judgement.meaningful
        } else {
            &judgement.shown
        };
        progress.limiter = shown.limiter;
        progress.limiter_text = shown.text.clone();
        progress.limiter_shares = judgement.shares.as_array();
        progress.link_bps = judgement.link_bps;
    }
}
