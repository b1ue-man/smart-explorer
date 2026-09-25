//! Schedule signals evaluated by the worker loop (`run_loop`): local tree
//! signatures, remote change tokens and removable-drive matching, plus the
//! process entry point that reads a pending handoff from the environment.

use crate::bisync::{DeletePolicy, Direction};
use crate::syncjobs::SyncJob;
use std::collections::HashSet;

use super::platform;
use super::run_loop::{run_daemon_with, Handoff};
use super::state::log;

/// A cheap signature of a local subtree: (file count, newest mtime ms, total
/// size). Any add/modify/delete changes at least one component.
pub(super) fn tree_sig(root: &std::path::Path) -> (u64, i64, u64) {
    let mut count = 0u64;
    let mut newest = 0i64;
    let mut bytes = 0u64;
    let mut stack = vec![root.to_path_buf()];
    let mut budget = 1_000_000u32; // bounded hint; the sync engine does a full checked walk
    while let Some(d) = stack.pop() {
        let rd = match std::fs::read_dir(&d) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for e in rd.flatten() {
            if budget == 0 {
                return (count, newest, bytes);
            }
            budget -= 1;
            let md = match std::fs::symlink_metadata(e.path()) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if platform::metadata_is_link_like(&md) {
                continue;
            }
            if md.is_dir() {
                stack.push(e.path());
            } else {
                count = count.saturating_add(1);
                bytes = bytes.saturating_add(md.len());
                if let Ok(t) = md.modified() {
                    if let Ok(d) = t.duration_since(std::time::UNIX_EPOCH) {
                        let ms = i64::try_from(d.as_millis()).unwrap_or(i64::MAX);
                        if ms > newest {
                            newest = ms;
                        }
                    }
                }
            }
        }
    }
    (count, newest, bytes)
}

/// Local filesystem root of an endpoint string, if it is a local path (not a
/// remote URL we can't watch).
pub(super) fn local_root(endpoint: &str) -> Option<std::path::PathBuf> {
    if endpoint.contains("://") {
        return None;
    }
    let p = std::path::PathBuf::from(endpoint);
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

/// Lightweight remote-change signal for realtime mirror jobs. It is only a
/// wakeup hint; the sync engine still validates the persisted cursor/state.
pub(super) fn remote_change_token(job: &SyncJob) -> Option<String> {
    if job.delete_policy != DeletePolicy::Mirror {
        return None;
    }
    let endpoint = match job.direction {
        Direction::AtoB => &job.source,
        Direction::BtoA => &job.target,
        Direction::Both => return None,
    };
    if local_root(endpoint).is_some() {
        return None;
    }
    let (backend, root) = crate::connect::resolve_endpoint(endpoint).ok()?;
    if !backend.supports_changes() {
        return None;
    }
    backend.current_change_cursor(&root).ok().flatten()
}

/// Set of currently-present removable-drive descriptors ("LETTER|LABEL|SERIAL").
pub(super) fn current_drives() -> HashSet<String> {
    platform::removable_drives()
        .into_iter()
        .map(|d| format!("{}|{}|{}", d.letter, d.label, d.serial))
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
    let parts: Vec<&str> = descriptor.split('|').collect();
    parts.iter().any(|p| wildcard_ci(pat, p))
}

/// Minimal case-insensitive glob (`*` and `?`).
pub(crate) fn wildcard_ci(pat: &str, s: &str) -> bool {
    let p: Vec<char> = pat.to_lowercase().chars().collect();
    let t: Vec<char> = s.to_lowercase().chars().collect();
    fn m(p: &[char], t: &[char]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some('*') => m(&p[1..], t) || (!t.is_empty() && m(p, &t[1..])),
            Some('?') => !t.is_empty() && m(&p[1..], &t[1..]),
            Some(&c) => !t.is_empty() && t[0] == c && m(&p[1..], &t[1..]),
        }
    }
    m(&p, &t)
}

/// Process entry (`--sync-daemon`): a replacement launched for a
/// version-upgrade handoff finds its generations in the environment.
pub fn run_daemon() {
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
