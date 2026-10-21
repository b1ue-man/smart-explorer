//! Who limits a transfer: the verdict from the time accounts of one window
//! (U44-6 point 2). Pure logic; the engine supplies the numbers, the displays
//! (desktop window, CLI, MCP, Android) show the result.
//!
//! Every operation books the time its thread spent reading the source, writing
//! the target, computing (hash, compression, encryption) and waiting for
//! permit, memory or folder. The side with the majority of the busy time is
//! the one the transfer waits for.

/// The resource that sets the pace of a transfer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Limiter {
    /// No verdict (yet, or nothing measured).
    #[default]
    None,
    /// The concurrency is still growing.
    Ramp,
    /// Demand does not fill the limit: the source delivers too slowly.
    Idle,
    /// A local source drive.
    Source,
    /// A local target drive.
    Target,
    /// The connection or the server of a remote side.
    Remote,
    /// The line itself: the rate is at the link rate.
    Link,
    /// Smart Explorer's own computing, every core in use.
    SeCpu,
    /// Smart Explorer computes on one thread while cores are free.
    SeThread,
    /// Waiting for the memory budget.
    Memory,
}

impl Limiter {
    /// The stable code of the MCP and bridge fields.
    pub fn code(self) -> &'static str {
        match self {
            Limiter::None => "none",
            Limiter::Ramp => "ramp",
            Limiter::Idle => "idle",
            Limiter::Source => "source",
            Limiter::Target => "target",
            Limiter::Remote => "remote",
            Limiter::Link => "link",
            Limiter::SeCpu => "se_cpu",
            Limiter::SeThread => "se_thread",
            Limiter::Memory => "memory",
        }
    }

    /// A verdict about a resource (not "none", "still ramping", "idle").
    pub fn names_a_resource(self) -> bool {
        !matches!(self, Limiter::None | Limiter::Ramp | Limiter::Idle)
    }
}

/// Time one operation spent, in nanoseconds, by account.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpAccounts {
    /// In read calls of the source.
    pub source_ns: u64,
    /// In write calls of the target.
    pub target_ns: u64,
    /// In hash, compression and encryption steps of the engine thread.
    pub cpu_ns: u64,
    /// Waiting for permit, memory or a folder.
    pub wait_ns: u64,
}

impl OpAccounts {
    /// The accounts of both.
    pub fn plus(self, other: OpAccounts) -> OpAccounts {
        OpAccounts {
            source_ns: self.source_ns.saturating_add(other.source_ns),
            target_ns: self.target_ns.saturating_add(other.target_ns),
            cpu_ns: self.cpu_ns.saturating_add(other.cpu_ns),
            wait_ns: self.wait_ns.saturating_add(other.wait_ns),
        }
    }
}

/// What one window of about a second showed. Rates are bytes per second.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WindowFacts {
    pub span_ms: u64,
    /// Demand filled the flow's limit for at least half the window.
    pub saturated: bool,
    /// The concurrency is still being raised (slow start, or the last upward
    /// probe paid off by 5 % or more).
    pub last_up_probe_gained: bool,
    pub source_ns: u64,
    pub target_ns: u64,
    pub cpu_ns: u64,
    pub wait_ns: u64,
    /// The part of `wait_ns` spent waiting for the memory budget.
    pub memory_wait_ns: u64,
    pub source_local: bool,
    pub target_local: bool,
    pub goodput_bps: u64,
    pub link_bps: Option<u64>,
    pub process_cpu_cores: Option<f64>,
    pub cores_available: Option<f64>,
}

/// The rate counts as the line's when it reaches 9/10 of the link rate:
/// TCP/IP, TLS and SSH framing take 3 to 5 %, the rest is margin.
const LINK_SHARE: (u128, u128) = (9, 10);
/// Own computing limits only when it keeps this share of the cores busy.
const CPU_SHARE: f64 = 0.9;
/// Windows with the same verdict before it is shown: with ±10 % noise per
/// window (TE1 simulation) a single window flips.
const CONFIRM_WINDOWS: u8 = 3;

/// The verdict of one window, by the rules of U44-6 point 2 in this order:
/// (a) demand does not fill the limit: `Idle`; (b) the concurrency still
/// grows: `Ramp`; (c) most of the time spent waiting, mostly for memory:
/// `Memory`; (d) the side with more than half of the busy time; (e) no such
/// side: `Remote`. A window without any measured time gives `None`.
pub fn judge(facts: &WindowFacts) -> Limiter {
    if !facts.saturated {
        return Limiter::Idle;
    }
    if facts.last_up_probe_gained {
        return Limiter::Ramp;
    }
    let busy = facts
        .source_ns
        .saturating_add(facts.target_ns)
        .saturating_add(facts.cpu_ns);
    let all = busy.saturating_add(facts.wait_ns);
    if all == 0 {
        return Limiter::None;
    }
    let waiting_mostly = facts.wait_ns.saturating_mul(2) > all;
    let memory_mostly = facts.memory_wait_ns.saturating_mul(2) > facts.wait_ns;
    if waiting_mostly && memory_mostly {
        return Limiter::Memory;
    }
    if busy == 0 {
        return Limiter::None;
    }
    let majority = |part: u64| part.saturating_mul(2) > busy;
    let verdict = if majority(facts.source_ns) {
        if facts.source_local {
            Limiter::Source
        } else {
            Limiter::Remote
        }
    } else if majority(facts.target_ns) {
        if facts.target_local {
            Limiter::Target
        } else {
            Limiter::Remote
        }
    } else if majority(facts.cpu_ns) {
        return own_computing(facts);
    } else {
        Limiter::Remote
    };
    if verdict == Limiter::Remote && at_link_rate(facts) {
        return Limiter::Link;
    }
    verdict
}

fn at_link_rate(facts: &WindowFacts) -> bool {
    facts.link_bps.filter(|link| *link > 0).is_some_and(|link| {
        u128::from(facts.goodput_bps) * LINK_SHARE.1 >= u128::from(link) * LINK_SHARE.0
    })
}

/// `SeCpu` when this process keeps (nearly) all its cores busy, `SeThread`
/// when it computes on fewer threads than it may use. Without both numbers
/// the claim "cores are free" cannot be made, so the computing is named.
fn own_computing(facts: &WindowFacts) -> Limiter {
    match (facts.process_cpu_cores, facts.cores_available) {
        (Some(used), Some(cores)) if used < CPU_SHARE * cores => Limiter::SeThread,
        _ => Limiter::SeCpu,
    }
}

/// Holds back a verdict until the same one came from three windows in a row.
#[derive(Clone, Copy, Debug, Default)]
pub struct Steady {
    confirmed: Limiter,
    pending: Limiter,
    streak: u8,
}

impl Steady {
    /// Takes the verdict of the next window; returns the one confirmed last
    /// (`None` before the first confirmation).
    pub fn push(&mut self, verdict: Limiter) -> Limiter {
        if self.streak > 0 && self.pending == verdict {
            self.streak = self.streak.saturating_add(1);
        } else {
            self.pending = verdict;
            self.streak = 1;
        }
        if self.streak >= CONFIRM_WINDOWS {
            self.confirmed = self.pending;
        }
        self.confirmed
    }
}

/// Where the time of the window went, in percent (the sum is 100 unless
/// nothing was measured).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shares {
    pub source: u8,
    pub target: u8,
    pub cpu: u8,
    pub wait: u8,
}

impl Shares {
    /// `[source, target, cpu, wait]`, the order of the progress field.
    pub fn as_array(self) -> [u8; 4] {
        [self.source, self.target, self.cpu, self.wait]
    }
}

/// Percentages rounded down; the remainder goes to the largest account.
pub fn shares(facts: &WindowFacts) -> Shares {
    let parts = [
        facts.source_ns,
        facts.target_ns,
        facts.cpu_ns,
        facts.wait_ns,
    ];
    let total: u128 = parts.iter().map(|part| u128::from(*part)).sum();
    if total == 0 {
        return Shares::default();
    }
    let mut percent = parts.map(|part| (u128::from(part) * 100 / total) as u8);
    let rest = 100 - percent.iter().map(|p| u32::from(*p)).sum::<u32>();
    let largest = (0..4)
        .max_by_key(|index| (parts[*index], 4 - *index))
        .unwrap_or(0);
    percent[largest] += rest as u8;
    Shares {
        source: percent[0],
        target: percent[1],
        cpu: percent[2],
        wait: percent[3],
    }
}

/// The names and numbers the texts quote, already formatted by the caller.
/// Only drives, devices and rates: never a file or folder name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextContext {
    /// The local source drive.
    pub source_name: String,
    /// The local target drive.
    pub target_name: String,
    /// The remote endpoint that limits.
    pub remote_name: String,
    /// "12.5 MB/s"
    pub rate: String,
    /// "125 MB/s"
    pub link: String,
}

fn with_name(text: &str, name: &str) -> String {
    if name.is_empty() {
        text.to_string()
    } else {
        format!("{text} ({name})")
    }
}

/// The German text of a verdict; `None` for "no verdict".
pub fn text(limiter: Limiter, ctx: &TextContext) -> Option<String> {
    Some(match limiter {
        Limiter::None => return None,
        Limiter::Source => with_name("Begrenzt durch das Laufwerk", &ctx.source_name),
        Limiter::Target => with_name("Begrenzt durch das Laufwerk", &ctx.target_name),
        Limiter::Remote => with_name("Begrenzt durch Leitung oder Server", &ctx.remote_name),
        Limiter::Link => format!("Begrenzt durch die Leitung ({} von {})", ctx.rate, ctx.link),
        Limiter::SeCpu => "Begrenzt durch Smart Explorer (Rechenzeit)".to_string(),
        Limiter::SeThread => {
            "Smart Explorer rechnet mit einem Faden; weitere Kerne sind frei".to_string()
        }
        Limiter::Memory => "Wartet auf Arbeitsspeicher (Puffer)".to_string(),
        Limiter::Ramp => "Läuft an …".to_string(),
        Limiter::Idle => "Wartet auf Nachschub (Quelle liefert nicht schnell genug)".to_string(),
    })
}

/// A local-to-local copy: both drives and what is known of their queues.
#[derive(Clone, Copy, Debug)]
pub struct LocalPair<'a> {
    pub source: &'a str,
    pub target: &'a str,
    pub same_volume: bool,
    pub source_queue: Option<f64>,
    pub target_queue: Option<f64>,
}

/// Which drive a local copy waits for. The copy is one kernel call for both
/// sides, so the time cannot be split; the queue depths decide when both are
/// known, otherwise both drives are named. Returns the verdict (`Source` or
/// `Target`) and the drive name for the text.
pub fn resolve_local_pair(verdict: Limiter, pair: &LocalPair<'_>) -> (Limiter, String) {
    if !matches!(verdict, Limiter::Source | Limiter::Target) {
        return (verdict, String::new());
    }
    if pair.same_volume {
        return (verdict, pair.source.to_string());
    }
    match (pair.source_queue, pair.target_queue) {
        (Some(source), Some(target)) if source > target => {
            (Limiter::Source, pair.source.to_string())
        }
        (Some(_), Some(_)) => (Limiter::Target, pair.target.to_string()),
        _ => (verdict, either_name(pair.source, pair.target)),
    }
}

/// "A oder B", or the one name that is known (empty when none is).
fn either_name(one: &str, other: &str) -> String {
    match (one.is_empty(), other.is_empty(), one == other) {
        (true, _, _) => other.to_string(),
        (_, true, _) | (_, _, true) => one.to_string(),
        _ => format!("{one} oder {other}"),
    }
}
