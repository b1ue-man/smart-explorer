//! Pure `/proc` logic of the Android exec host (`exec_contain.rs`): parsing
//! `/proc/<pid>/stat`, the descendants of a job's intermediate, what one kill
//! pass signals, and the crash record of a job root. No I/O happens here, so
//! the Linux host tests cover it; the Android adapter feeds it `/proc`.
use std::collections::{BTreeMap, BTreeSet};

/// The fields of `/proc/<pid>/stat` the containment needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProcStat {
    pub(crate) pid: i32,
    /// Field 3: `R`, `S`, `D`, `Z` (zombie), …
    pub(crate) state: u8,
    pub(crate) ppid: i32,
    pub(crate) pgrp: i32,
    /// Field 22, clock ticks after boot: tells a reused pid apart.
    pub(crate) start_time: u64,
}

impl ProcStat {
    fn is_zombie(&self) -> bool {
        self.state == b'Z'
    }
}

/// Parses one `/proc/<pid>/stat` line. The command name (field 2) may hold
/// spaces and `)`, so every later field is read after the last `)`.
pub(crate) fn parse_stat(text: &str) -> Option<ProcStat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    if close < open {
        return None;
    }
    let pid = text[..open].trim().parse().ok()?;
    let mut fields = text[close + 1..].split_ascii_whitespace();
    let state = match fields.next()?.as_bytes() {
        [state] => *state,
        _ => return None,
    };
    let ppid = fields.next()?.parse().ok()?;
    let pgrp = fields.next()?.parse().ok()?;
    // Fields 6 (session) to 21 (itrealvalue) are skipped; field 22 follows.
    let start_time = fields.nth(16)?.parse().ok()?;
    Some(ProcStat {
        pid,
        state,
        ppid,
        pgrp,
        start_time,
    })
}

/// Every pid below `root` in `table` (children, their children, …); `root`
/// itself is not part of it. A cycle in a torn snapshot cannot loop.
pub(crate) fn descendants(table: &[ProcStat], root: i32) -> BTreeSet<i32> {
    let mut children: BTreeMap<i32, Vec<i32>> = BTreeMap::new();
    for stat in table {
        children.entry(stat.ppid).or_default().push(stat.pid);
    }
    let mut found = BTreeSet::new();
    let mut queue = vec![root];
    while let Some(parent) = queue.pop() {
        for &child in children.get(&parent).into_iter().flatten() {
            if child != root && found.insert(child) {
                queue.push(child);
            }
        }
    }
    found
}

/// What one kill pass signals with SIGKILL.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct KillPlan {
    /// Live (non-zombie) descendants of the intermediate.
    pub(crate) pids: Vec<i32>,
    /// Process groups led by a live descendant: a leader alive in the
    /// snapshot keeps the group id from belonging to anybody else, and one
    /// group signal also reaches members forked after the snapshot.
    pub(crate) groups: Vec<i32>,
}

/// The kill pass for the tree below `intermediate`. The intermediate itself
/// (it must outlive its tree so orphans stay below it), this app process
/// (`own`) and pids up to 1 are never targets.
pub(crate) fn kill_plan(table: &[ProcStat], intermediate: i32, own: i32) -> KillPlan {
    let below = descendants(table, intermediate);
    let target = |pid: i32| pid > 1 && pid != own && pid != intermediate;
    let live: BTreeSet<i32> = table
        .iter()
        .filter(|stat| below.contains(&stat.pid) && !stat.is_zombie() && target(stat.pid))
        .map(|stat| stat.pid)
        .collect();
    let groups: BTreeSet<i32> = table
        .iter()
        .filter(|stat| live.contains(&stat.pid) && live.contains(&stat.pgrp))
        .map(|stat| stat.pgrp)
        .collect();
    KillPlan {
        pids: live.into_iter().collect(),
        groups: groups.into_iter().collect(),
    }
}

/// Crash record of a job root: the intermediate's pid and start time.
pub(crate) fn record_text(pid: i32, start_time: u64) -> String {
    format!("{pid} {start_time}\n")
}

/// Reads a crash record; anything else than `<pid> <start time>` is `None`.
pub(crate) fn parse_record(text: &str) -> Option<(i32, u64)> {
    let mut parts = text.split_ascii_whitespace();
    let pid: i32 = parts.next()?.parse().ok()?;
    let start_time = parts.next()?.parse().ok()?;
    (parts.next().is_none() && pid > 1).then_some((pid, start_time))
}

/// The recorded job root still runs: the same pid with the same start time
/// (a reused pid has a later one) and not merely a zombie.
pub(crate) fn is_recorded_root(stat: Option<&ProcStat>, pid: i32, start_time: u64) -> bool {
    stat.is_some_and(|stat| stat.pid == pid && stat.start_time == start_time && !stat.is_zombie())
}
