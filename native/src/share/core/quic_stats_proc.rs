//! What the operating system adds to the QUIC statistics of a transfer burst
//! (diagnosis of the speed stage, E13): the UDP counters of the kernel (a
//! full receive buffer shows as `RcvbufErrors`), the limits of the socket
//! buffers, and the CPU time of the process and of its busiest threads.
//!
//! The sources are text files of the Linux `/proc` tree; where a file does not
//! exist (any other system) its values are absent and the line says so. The
//! parsers are pure so a unit test can feed them.
use std::collections::HashMap;

use serde::Serialize;

/// One tick of the Linux process clock in milliseconds (`USER_HZ` is 100 on
/// every Linux ABI).
const MS_PER_CLOCK_TICK: u64 = 10;

/// The `Udp:` row of `/proc/net/snmp` (system-wide counters).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(super) struct UdpCounters {
    pub(super) in_datagrams: u64,
    pub(super) no_ports: u64,
    pub(super) in_errors: u64,
    pub(super) out_datagrams: u64,
    /// Datagrams the kernel dropped because the receive buffer of the socket
    /// was full: the loss a congestion controller reads as congestion.
    pub(super) rcvbuf_errors: u64,
    pub(super) sndbuf_errors: u64,
    pub(super) in_csum_errors: u64,
}

impl UdpCounters {
    pub(super) fn since(self, earlier: Self) -> Self {
        Self {
            in_datagrams: self.in_datagrams.saturating_sub(earlier.in_datagrams),
            no_ports: self.no_ports.saturating_sub(earlier.no_ports),
            in_errors: self.in_errors.saturating_sub(earlier.in_errors),
            out_datagrams: self.out_datagrams.saturating_sub(earlier.out_datagrams),
            rcvbuf_errors: self.rcvbuf_errors.saturating_sub(earlier.rcvbuf_errors),
            sndbuf_errors: self.sndbuf_errors.saturating_sub(earlier.sndbuf_errors),
            in_csum_errors: self.in_csum_errors.saturating_sub(earlier.in_csum_errors),
        }
    }
}

/// The counters of the `Udp:` row; columns are matched by name, so a kernel
/// that adds or reorders columns still parses. `None` without such a row.
pub(super) fn parse_udp_snmp(text: &str) -> Option<UdpCounters> {
    let mut rows = text.lines().filter(|line| line.starts_with("Udp:"));
    let names = rows.next()?;
    let values = rows.next()?;
    let table: HashMap<&str, u64> = names
        .split_whitespace()
        .skip(1)
        .zip(values.split_whitespace().skip(1))
        .filter_map(|(name, value)| Some((name, value.parse().ok()?)))
        .collect();
    let get = |name: &str| table.get(name).copied().unwrap_or(0);
    Some(UdpCounters {
        in_datagrams: get("InDatagrams"),
        no_ports: get("NoPorts"),
        in_errors: get("InErrors"),
        out_datagrams: get("OutDatagrams"),
        rcvbuf_errors: get("RcvbufErrors"),
        sndbuf_errors: get("SndbufErrors"),
        in_csum_errors: get("InCsumErrors"),
    })
}

/// The name and the CPU time (user plus system, milliseconds) in the text of a
/// `/proc/<pid>/stat` or `/proc/<pid>/task/<tid>/stat`. The name sits between
/// the first `(` and the last `)` and may contain spaces and parentheses.
pub(super) fn parse_stat_cpu(text: &str) -> Option<(String, u64)> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    if close < open {
        return None;
    }
    let name = text[open + 1..close].to_string();
    // After the name: state (field 3), then fields 4 and following; utime and
    // stime are fields 14 and 15, so the 12th and 13th after the state.
    let fields: Vec<&str> = text[close + 1..].split_whitespace().collect();
    let user: u64 = fields.get(11)?.parse().ok()?;
    let system: u64 = fields.get(12)?.parse().ok()?;
    Some((
        name,
        user.saturating_add(system)
            .saturating_mul(MS_PER_CLOCK_TICK),
    ))
}

/// A thread and the CPU time it has used so far.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ThreadCpu {
    pub(super) tid: u64,
    pub(super) name: String,
    pub(super) cpu_ms: u64,
}

/// The threads that used the most CPU time between `before` and `after`
/// (threads that began in between count from zero), at most `limit`, busiest
/// first; threads without CPU time are left out.
pub(super) fn busiest_threads(
    before: &[ThreadCpu],
    after: &[ThreadCpu],
    limit: usize,
) -> Vec<ThreadCpu> {
    let earlier: HashMap<u64, u64> = before.iter().map(|t| (t.tid, t.cpu_ms)).collect();
    let mut used: Vec<ThreadCpu> = after
        .iter()
        .map(|thread| ThreadCpu {
            tid: thread.tid,
            name: thread.name.clone(),
            cpu_ms: thread
                .cpu_ms
                .saturating_sub(earlier.get(&thread.tid).copied().unwrap_or(0)),
        })
        .filter(|thread| thread.cpu_ms > 0)
        .collect();
    used.sort_by(|a, b| b.cpu_ms.cmp(&a.cpu_ms).then(a.tid.cmp(&b.tid)));
    used.truncate(limit);
    used
}

/// UDP counters of the system, if the system has them.
pub(super) fn read_udp_counters() -> Option<UdpCounters> {
    parse_udp_snmp(&std::fs::read_to_string("/proc/net/snmp").ok()?)
}

/// CPU time of this process in milliseconds, if the system has it.
pub(super) fn read_process_cpu_ms() -> Option<u64> {
    parse_stat_cpu(&std::fs::read_to_string("/proc/self/stat").ok()?).map(|(_, ms)| ms)
}

/// CPU time of every thread of this process.
pub(super) fn read_thread_cpu() -> Vec<ThreadCpu> {
    let Ok(entries) = std::fs::read_dir("/proc/self/task") else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let tid: u64 = entry.file_name().to_str()?.parse().ok()?;
            let text = std::fs::read_to_string(entry.path().join("stat")).ok()?;
            let (name, cpu_ms) = parse_stat_cpu(&text)?;
            Some(ThreadCpu { tid, name, cpu_ms })
        })
        .collect()
}

/// `net.core.rmem_max` and `net.core.wmem_max` (bytes): the ceilings of the
/// socket buffers an unprivileged process can request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(super) struct BufferLimits {
    pub(super) rmem_max: Option<u64>,
    pub(super) wmem_max: Option<u64>,
}

pub(super) fn read_buffer_limits() -> BufferLimits {
    let read = |name: &str| {
        std::fs::read_to_string(format!("/proc/sys/net/core/{name}"))
            .ok()
            .and_then(|text| text.trim().parse().ok())
    };
    BufferLimits {
        rmem_max: read("rmem_max"),
        wmem_max: read("wmem_max"),
    }
}
