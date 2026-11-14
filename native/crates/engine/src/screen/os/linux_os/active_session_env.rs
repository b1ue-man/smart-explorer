//! The environment of the followed session (E23, S3), read only from
//! processes whose real, effective, saved and filesystem UID are the
//! session's. Processes in the session's own scope (`session-<id>.scope`)
//! answer first; only when none of them names the display do the account's
//! other processes (GNOME runs its shell in the user manager, not in the
//! scope). The answering processes must agree; a second value is
//! ambiguous and gives no environment. X11 takes `DISPLAY` exactly as
//! logind names it and an `XAUTHORITY` only when that file is a regular
//! file of the session's account; Wayland takes `WAYLAND_DISPLAY` only as a
//! socket of the account in its logind runtime directory, and the session
//! bus of the same processes (or the account's `bus` socket there).
use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::os::unix::fs::{FileTypeExt, MetadataExt};

use super::{LogindSession, SessionKind};

/// Largest environment block read from one process.
const MAX_ENVIRON: u64 = 1024 * 1024;
/// Variables taken over from the answering processes when they agree.
const DESKTOP_VARIABLES: [&str; 3] = ["XDG_CURRENT_DESKTOP", "DESKTOP_SESSION", "LANG"];

/// What `/proc` and the file system say about processes and files.
pub trait ProcessTable {
    fn pids(&self) -> Vec<u32>;
    /// Real, effective, saved and filesystem UID.
    fn uids(&self, pid: u32) -> Option<[u32; 4]>;
    fn cgroup(&self, pid: u32) -> Option<String>;
    /// The NUL-separated environment block.
    fn environ(&self, pid: u32) -> Option<Vec<u8>>;
    /// Owner of `path` when it is a regular file (links are not followed).
    fn file_owner(&self, path: &str) -> Option<u32>;
    /// Owner of `path` when it is a socket (links are not followed).
    fn socket_owner(&self, path: &str) -> Option<u32>;
    /// Owner of `path` when it is a directory (links are not followed).
    fn dir_owner(&self, path: &str) -> Option<u32>;
}

/// The processes and files of this machine.
pub struct SystemProcesses;

fn owner_if(path: &str, wanted: impl Fn(&fs::FileType) -> bool) -> Option<u32> {
    let metadata = fs::symlink_metadata(path).ok()?;
    wanted(&metadata.file_type()).then(|| metadata.uid())
}

impl ProcessTable for SystemProcesses {
    fn pids(&self) -> Vec<u32> {
        let Ok(entries) = fs::read_dir("/proc") else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
            .collect()
    }
    fn uids(&self, pid: u32) -> Option<[u32; 4]> {
        let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        let line = status.lines().find_map(|line| line.strip_prefix("Uid:"))?;
        let ids: Vec<u32> = line
            .split_whitespace()
            .map(|id| id.parse().ok())
            .collect::<Option<_>>()?;
        <[u32; 4]>::try_from(ids).ok()
    }
    fn cgroup(&self, pid: u32) -> Option<String> {
        fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()
    }
    fn environ(&self, pid: u32) -> Option<Vec<u8>> {
        let mut block = Vec::new();
        fs::File::open(format!("/proc/{pid}/environ"))
            .ok()?
            .take(MAX_ENVIRON + 1)
            .read_to_end(&mut block)
            .ok()?;
        ((block.len() as u64) <= MAX_ENVIRON).then_some(block)
    }
    fn file_owner(&self, path: &str) -> Option<u32> {
        owner_if(path, fs::FileType::is_file)
    }
    fn socket_owner(&self, path: &str) -> Option<u32> {
        owner_if(path, FileTypeExt::is_socket)
    }
    fn dir_owner(&self, path: &str) -> Option<u32> {
        owner_if(path, fs::FileType::is_dir)
    }
}

/// The value of `name` in a NUL-separated environment block.
pub fn env_value(block: &[u8], name: &str) -> Option<String> {
    block
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(name.as_bytes())?.strip_prefix(b"="))
        .map(|value| String::from_utf8_lossy(value).into_owned())
}

/// Whether a `/proc/<pid>/cgroup` text places the process in the scope of
/// logind session `session`.
pub fn in_session_scope(cgroup: &str, session: &str) -> bool {
    let scope = format!("/session-{session}.scope");
    cgroup.lines().any(|line| line.ends_with(&scope))
}

/// A local X11 display name (`:N` or `:N.S`).
pub fn local_display(value: &str) -> bool {
    value.strip_prefix(':').is_some_and(|rest| {
        !rest.is_empty()
            && rest.len() <= 20
            && rest.bytes().all(|b| b.is_ascii_digit() || b == b'.')
    })
}

/// The environments of the account's processes, scope first.
fn environments(table: &dyn ProcessTable, session: &LogindSession) -> [Vec<Vec<u8>>; 2] {
    let mut scope = Vec::new();
    let mut account = Vec::new();
    let owned = |pid: u32| table.uids(pid) == Some([session.uid; 4]);
    for pid in table.pids() {
        if !owned(pid) {
            continue;
        }
        let in_scope = table
            .cgroup(pid)
            .is_some_and(|cgroup| in_session_scope(&cgroup, &session.id));
        let Some(block) = table.environ(pid) else {
            continue;
        };
        // The PID may have been reused between the checks and the read.
        if !owned(pid) {
            continue;
        }
        if in_scope {
            scope.push(block);
        } else {
            account.push(block);
        }
    }
    [scope, account]
}

/// The one value of `name` among `blocks`; `Err` when they disagree.
fn agreed(blocks: &[&Vec<u8>], name: &str) -> Result<Option<String>, ()> {
    let values: BTreeSet<String> = blocks
        .iter()
        .filter_map(|block| env_value(block.as_slice(), name))
        .filter(|value| !value.is_empty())
        .collect();
    match values.len() {
        0 => Ok(None),
        1 => Ok(values.into_iter().next()),
        _ => Err(()),
    }
}

/// The processes of the first tier for which `names_display` holds.
fn answering<'a>(
    tiers: &'a [Vec<Vec<u8>>; 2],
    names_display: impl Fn(&[u8]) -> bool,
) -> Vec<&'a Vec<u8>> {
    tiers
        .iter()
        .map(|tier| {
            tier.iter()
                .filter(|block| names_display(block.as_slice()))
                .collect::<Vec<_>>()
        })
        .find(|found| !found.is_empty())
        .unwrap_or_default()
}

/// The variables a process started in `session` gets, or why there are
/// none (a short reason for the report of the worker).
pub fn collect(
    table: &dyn ProcessTable,
    session: &LogindSession,
    kind: SessionKind,
) -> Result<Vec<(String, String)>, &'static str> {
    let runtime = session.runtime_path.as_str();
    if !runtime.starts_with('/') || table.dir_owner(runtime) != Some(session.uid) {
        return Err("runtime_dir_not_owned");
    }
    let tiers = environments(table, session);
    let mut env = vec![
        ("XDG_RUNTIME_DIR".to_string(), runtime.to_string()),
        ("XDG_SESSION_ID".to_string(), session.id.clone()),
    ];
    let found = match kind {
        SessionKind::X11 => {
            if !local_display(&session.display) {
                return Err("display_not_local");
            }
            let found = answering(&tiers, |block| {
                env_value(block, "DISPLAY").as_deref() == Some(session.display.as_str())
            });
            if found.is_empty() {
                return Err("display_without_process");
            }
            env.push(("XDG_SESSION_TYPE".into(), "x11".into()));
            env.push(("DISPLAY".into(), session.display.clone()));
            match agreed(&found, "XAUTHORITY").map_err(|()| "xauthority_ambiguous")? {
                Some(path) if !path.starts_with('/') => return Err("xauthority_not_absolute"),
                Some(path) if table.file_owner(&path) != Some(session.uid) => {
                    return Err("xauthority_foreign")
                }
                Some(path) => env.push(("XAUTHORITY".into(), path)),
                None => {}
            }
            found
        }
        SessionKind::Wayland => {
            let found = answering(&tiers, |block| {
                env_value(block, "XDG_RUNTIME_DIR").as_deref() == Some(runtime)
                    && env_value(block, "WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty())
            });
            let display = agreed(&found, "WAYLAND_DISPLAY")
                .map_err(|()| "wayland_display_ambiguous")?
                .ok_or("wayland_display_missing")?;
            let socket = if display.starts_with('/') {
                display.clone()
            } else if display.contains('/') || display == "." || display == ".." {
                return Err("wayland_display_invalid");
            } else {
                format!("{runtime}/{display}")
            };
            if !socket.starts_with(&format!("{runtime}/"))
                || table.socket_owner(&socket) != Some(session.uid)
            {
                return Err("wayland_socket_foreign");
            }
            let bus = match agreed(&found, "DBUS_SESSION_BUS_ADDRESS")
                .map_err(|()| "session_bus_ambiguous")?
            {
                Some(bus) => bus,
                None => {
                    let socket = format!("{runtime}/bus");
                    if table.socket_owner(&socket) != Some(session.uid) {
                        return Err("session_bus_missing");
                    }
                    format!("unix:path={socket}")
                }
            };
            env.push(("XDG_SESSION_TYPE".into(), "wayland".into()));
            env.push(("WAYLAND_DISPLAY".into(), display));
            env.push(("DBUS_SESSION_BUS_ADDRESS".into(), bus));
            found
        }
        SessionKind::None => return Err("not_graphical"),
    };
    if kind == SessionKind::X11 {
        if let Ok(Some(bus)) = agreed(&found, "DBUS_SESSION_BUS_ADDRESS") {
            env.push(("DBUS_SESSION_BUS_ADDRESS".into(), bus));
        }
    }
    for name in DESKTOP_VARIABLES {
        if let Ok(Some(value)) = agreed(&found, name) {
            env.push((name.to_string(), value));
        }
    }
    Ok(env)
}
