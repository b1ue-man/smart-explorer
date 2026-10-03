//! Schedule signals evaluated by the worker loop (`run_loop`): local tree
//! signatures, remote change tokens and removable-drive matching, plus the
//! process entry point that reads a pending handoff from the environment.

use std::collections::HashSet;

use super::platform;
use super::run_loop::{run_daemon_with, Handoff};
use super::state::log;

/// Parse with the shared location contract, without network I/O or requiring
/// existence: a missing local root must be rearmed when it returns.
pub(super) fn local_root(endpoint: &str) -> Option<std::path::PathBuf> {
    crate::connect::local_endpoint_path(endpoint).ok().flatten().map(Into::into)
}

pub fn next_scheduled_run(now: i64) -> Option<i64> {
    let report = crate::syncjobs::load_report().ok()?;
    let states = crate::syncjobs::load_job_states(&report.jobs);
    report.jobs.iter().filter_map(|job| states.get(&job.id)
        .filter(|state| state.load_error.is_none())
        .and_then(|state| super::due::next_due(job, state, now))).min()
}

/// Set of currently-present removable-drive descriptors ("LETTER|LABEL|SERIAL").
pub(super) fn current_drives() -> HashSet<String> {
    platform::removable_drives()
        .into_iter()
        .filter_map(|d| serde_json::to_string(&(d.letter, d.label, d.serial)).ok())
        .collect()
}

/// Does a drive descriptor match a job's `connect_match` (empty = any removable;
/// otherwise a case-insensitive `*?` wildcard tested against letter, label and
/// serial)?
pub(crate) fn drive_matches(pat: &str, descriptor: &str) -> bool {
    let pat = pat.trim();
    if pat.is_empty() {
        return true;
    }
    let parts: Vec<String> = serde_json::from_str::<(String, String, String)>(descriptor)
        .map(|(root, label, serial)| vec![root, label, serial])
        .unwrap_or_else(|_| descriptor.split('|').map(str::to_string).collect());
    parts.iter().any(|part| wildcard_ci(pat, part) || (part.starts_with(r"\\?\Volume{")
        && part.rsplit_once(':').is_some_and(|(_, serial)| wildcard_ci(pat, serial))))
}

/// Minimal case-insensitive glob (`*` and `?`).
pub(crate) fn wildcard_ci(pat: &str, s: &str) -> bool {
    let p: Vec<char> = pat.to_lowercase().chars().collect();
    let t: Vec<char> = s.to_lowercase().chars().collect();
    let (mut pi, mut ti, mut star, mut consumed) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) { pi += 1; ti += 1; }
        else if pi < p.len() && p[pi] == '*' { star = Some(pi); pi += 1; consumed = ti; }
        else if let Some(at) = star { consumed += 1; ti = consumed; pi = at + 1; }
        else { return false; }
    }
    while pi < p.len() && p[pi] == '*' { pi += 1; }
    pi == p.len()
}

/// Process entry (`--sync-daemon`): a replacement launched for a
/// version-upgrade handoff finds its generations in the environment.
pub fn run_daemon() {
    if !super::guardian::is_child() { super::guardian::run_guardian(); return; }
    let handoff_generation = std::env::var_os(crate::autostart::DAEMON_HANDOFF_ENV);
    let retiring_generation = std::env::var_os(crate::autostart::DAEMON_RETIRING_GENERATION_ENV);
    std::env::remove_var(crate::autostart::DAEMON_HANDOFF_ENV);
    std::env::remove_var(crate::autostart::DAEMON_RETIRING_GENERATION_ENV);
    let handoff_generation = match handoff_generation {
        Some(value) => {
            let Some(value) = value.to_str().filter(|value| valid_generation(value)) else {
                log("daemon refused invalid handoff generation");
                return;
            };
            Some(value.to_string())
        }
        None => None,
    };
    let retiring_generation = match retiring_generation {
        Some(value) => {
            let Some(value) = value.to_str().filter(|value| valid_generation(value)) else {
                log("daemon refused invalid retiring generation");
                return;
            };
            Some(value.to_string())
        }
        None => None,
    };
    run_daemon_with(handoff_generation.map(|generation| Handoff {
        generation,
        retiring_generation,
    }));
}

fn valid_generation(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn new_generation() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|error| error.to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(super) fn drive_root(descriptor: &str) -> Option<String> {
    serde_json::from_str::<(String, String, String)>(descriptor).ok().map(|tuple| tuple.0)
        .or_else(|| descriptor.split('|').next().map(str::to_string))
}
