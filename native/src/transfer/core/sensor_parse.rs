//! Pure parsers and arithmetic of the system sensors (`os/shared/sensors.rs`
//! reads, this file interprets). Every reader answers `None` for a source
//! that is missing, forbidden or unreadable: an unknown value is not an error
//! (Android refuses apps `/proc/stat`; `/sys/class/net/*/speed` answers
//! `EINVAL` on an unplugged link), and nothing here needs any value.

// The Linux readers are not called on Windows (their parsers are pure and
// tested everywhere).
#![cfg_attr(windows, allow(dead_code))]

/// CPU ticks of the whole machine: `busy` out of `total`, in any unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuTicks {
    pub busy: u64,
    pub total: u64,
}

/// Kernel counters of one block device (fields 10 and 11 of
/// `/proc/diskstats`, Documentation/admin-guide/iostats.rst).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiskTicks {
    /// Milliseconds the device had I/O in flight.
    pub io_ms: u64,
    /// Milliseconds weighted by the number of requests in flight.
    pub weighted_ms: u64,
}

/// What one reading of the system gave; `None` is "could not read".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawSample {
    pub cpu: Option<CpuTicks>,
    /// CPU time of this process (user + system) in nanoseconds.
    pub process_ns: Option<u64>,
    /// Cores this process may use (affinity and cgroup quota).
    pub cores: Option<f64>,
    pub mem_available: Option<u64>,
    pub threads_free: Option<u64>,
    pub link_bps: Option<u64>,
    /// The watched devices by id (`major:minor`).
    pub disks: Vec<(String, DiskTicks)>,
}

/// The numbers the limiter and the displays work with (U44-6 point 3). Not
/// every field has a reader yet: the share and agent compression policy and
/// the displays of the CLI and Android take what they need.
#[derive(Clone, Debug, Default, PartialEq)]
#[allow(dead_code)]
pub struct SystemSample {
    /// Share of the machine's CPU time in use (0..=1).
    pub cpu_busy: Option<f64>,
    /// Cores this process may use; the machine's parallelism when the
    /// affinity cannot be read.
    pub cores_available: f64,
    /// Cores' worth of CPU time this process used since the last sample;
    /// meaningful only when `process_cpu_known`.
    pub process_cpu_cores: f64,
    pub process_cpu_known: bool,
    pub mem_available: Option<u64>,
    pub threads_free: Option<u64>,
    /// Link rate of the interface of the default route in bytes per second
    /// (like every `*_bps` of the transfers).
    pub link_bps: Option<u64>,
    /// Longest queue among the watched devices.
    pub disk_queue: Option<f64>,
    /// Queue depth per watched device id (`major:minor`).
    pub disk_queues: Vec<(String, f64)>,
}

#[allow(dead_code)]
impl SystemSample {
    /// Cores this process does not use, when both numbers are known.
    pub fn free_cores(&self) -> Option<f64> {
        self.process_cpu_known
            .then(|| (self.cores_available - self.process_cpu_cores).max(0.0))
    }
}

/// `cpu user nice system idle iowait irq softirq steal …` of `/proc/stat`;
/// guest time is part of user time already.
pub fn parse_proc_stat(text: &str) -> Option<CpuTicks> {
    let line = text.lines().next()?;
    let mut fields = line.split_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    let numbers: Vec<u64> = fields
        .take(8)
        .map(|f| f.parse().ok())
        .collect::<Option<_>>()?;
    if numbers.len() < 4 {
        return None;
    }
    let total: u64 = numbers.iter().sum();
    let idle = numbers[3].saturating_add(numbers.get(4).copied().unwrap_or(0));
    Some(CpuTicks {
        busy: total.saturating_sub(idle),
        total,
    })
}

/// Share of the time between two readings the machine was busy.
pub fn busy_fraction(prev: CpuTicks, next: CpuTicks) -> Option<f64> {
    let total = next.total.checked_sub(prev.total).filter(|t| *t > 0)?;
    let busy = next.busy.saturating_sub(prev.busy);
    Some((busy as f64 / total as f64).clamp(0.0, 1.0))
}

/// `MemAvailable:` of `/proc/meminfo`, in bytes.
pub fn parse_meminfo_available(text: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.starts_with("MemAvailable:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

/// `cpu.max` of a cgroup v2: `max <period>` leaves the cores of the
/// affinity, `<quota> <period>` limits them to quota ÷ period.
pub fn parse_cpu_max(text: &str, affinity_cores: usize) -> f64 {
    let affinity = affinity_cores.max(1) as f64;
    let mut fields = text.split_whitespace();
    let (Some(quota), Some(period)) = (fields.next(), fields.next()) else {
        return affinity;
    };
    match (quota.parse::<f64>(), period.parse::<f64>()) {
        (Ok(quota), Ok(period)) if quota > 0.0 && period > 0.0 => affinity.min(quota / period),
        _ => affinity,
    }
}

/// Threads the cgroup still admits: `pids.max` − `pids.current`; `None` for
/// "max" (no limit) and for anything unreadable.
pub fn parse_pids(max_text: &str, current_text: &str) -> Option<u64> {
    let max: u64 = max_text.trim().parse().ok()?;
    let current: u64 = current_text.trim().parse().ok()?;
    Some(max.saturating_sub(current))
}

/// `utime + stime` in clock ticks of `/proc/self/stat` (fields 14 and 15;
/// the command name in parentheses may hold spaces, so counting starts after
/// its closing bracket).
pub fn parse_process_ticks(text: &str) -> Option<u64> {
    let after = &text[text.rfind(')')? + 1..];
    let fields: Vec<&str> = after.split_whitespace().collect();
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    utime.checked_add(stime)
}

/// The row of `/proc/diskstats` whose name or `major:minor` is `device`:
/// its name and counters.
pub fn find_disk(text: &str, device: &str) -> Option<(String, DiskTicks)> {
    text.lines().find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 14 {
            return None;
        }
        let id = format!("{}:{}", fields[0], fields[1]);
        if fields[2] != device && id != device {
            return None;
        }
        Some((
            fields[2].to_string(),
            DiskTicks {
                io_ms: fields[12].parse().ok()?,
                weighted_ms: fields[13].parse().ok()?,
            },
        ))
    })
}

/// Counters of `device` (name or `major:minor`).
pub fn parse_diskstats(text: &str, device: &str) -> Option<DiskTicks> {
    find_disk(text, device).map(|(_, ticks)| ticks)
}

/// Average requests in flight between two readings: the weighted time grew
/// by Δweighted while Δwall passed (not "%util", which saturates at one
/// request on devices that serve many at once).
pub fn queue_depth(prev: DiskTicks, next: DiskTicks, wall_ms: u64) -> f64 {
    if wall_ms == 0 {
        return 0.0;
    }
    next.weighted_ms.saturating_sub(prev.weighted_ms) as f64 / wall_ms as f64
}

/// The interface of the default route in `/proc/net/route` (destination and
/// mask 0, route up; the lowest metric wins).
pub fn parse_default_route(text: &str) -> Option<String> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let flags = u32::from_str_radix(f.get(3)?, 16).ok()?;
            let metric: u64 = f.get(6)?.parse().ok()?;
            (*f.get(1)? == "00000000" && *f.get(7)? == "00000000" && flags & 1 != 0)
                .then(|| (metric, f[0].to_string()))
        })
        .min_by_key(|(metric, _)| *metric)
        .map(|(_, name)| name)
}

/// `/sys/class/net/<if>/speed` (Mbit/s; -1 when unknown) in bytes per second,
/// the unit of the transfer rates it is compared with.
pub fn parse_link_speed(text: &str) -> Option<u64> {
    let mbit: i64 = text.trim().parse().ok()?;
    u64::try_from(mbit)
        .ok()
        .filter(|m| *m > 0)
        .map(|m| m.saturating_mul(125_000))
}

/// Interprets two readings `wall_ms` apart. Without an earlier reading only
/// the values that need none are filled in.
pub fn sample_from(prev: Option<&RawSample>, next: &RawSample, wall_ms: u64) -> SystemSample {
    let mut sample = SystemSample {
        cores_available: next.cores.unwrap_or_else(fallback_cores),
        mem_available: next.mem_available,
        threads_free: next.threads_free,
        link_bps: next.link_bps,
        ..SystemSample::default()
    };
    let Some(prev) = prev else {
        return sample;
    };
    if let (Some(before), Some(after)) = (prev.cpu, next.cpu) {
        sample.cpu_busy = busy_fraction(before, after);
    }
    if let (Some(before), Some(after)) = (prev.process_ns, next.process_ns) {
        if wall_ms > 0 {
            sample.process_cpu_cores =
                after.saturating_sub(before) as f64 / (wall_ms as f64 * 1_000_000.0);
            sample.process_cpu_known = true;
        }
    }
    for (id, after) in &next.disks {
        if let Some((_, before)) = prev.disks.iter().find(|(known, _)| known == id) {
            sample
                .disk_queues
                .push((id.clone(), queue_depth(*before, *after, wall_ms)));
        }
    }
    sample.disk_queue = sample.disk_queues.iter().map(|(_, q)| *q).reduce(f64::max);
    sample
}

fn fallback_cores() -> f64 {
    std::thread::available_parallelism().map_or(1.0, |n| n.get() as f64)
}
