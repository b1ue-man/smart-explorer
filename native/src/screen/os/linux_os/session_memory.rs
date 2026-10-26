//! Linux admission facts: physical headroom, cgroup ancestry and process limits.
use crate::screen::core::frame_memory::MemoryFacts;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
pub(super) fn preview_admission() -> crate::screen::core::frame_budget::PreviewAdmission {
    crate::screen::core::frame_budget::PreviewAdmission::FullContract
}
pub(super) fn ceiling(
    facts: Option<MemoryFacts>,
) -> crate::screen::core::frame_memory::CapacityCeiling {
    crate::screen::core::frame_memory::CapacityCeiling::desktop(facts)
}
pub(super) fn observe_ceiling(_ceiling: crate::screen::core::frame_memory::CapacityCeiling) {}
use crate::screen::core::frame_budget::MAX_WORKING_BYTES;

pub(super) fn facts() -> Option<MemoryFacts> {
    let mem = std::fs::read_to_string("/proc/meminfo").ok()?;
    let status = std::fs::read_to_string("/proc/self/status").ok();
    let physical = kib(&mem, "MemAvailable:")?;
    // Unknown constraints permit only the prior class, never an extension.
    let fallback = MAX_WORKING_BYTES * 4;
    let group = cgroup_headroom().unwrap_or(fallback);
    let address = status
        .as_ref()
        .and_then(|text| kib(text, "VmSize:"))
        .and_then(|used| remaining_limit(true, used))
        .unwrap_or(fallback);
    let data = status
        .as_ref()
        .and_then(|text| kib(text, "VmData:"))
        .and_then(|used| remaining_limit(false, used))
        .unwrap_or(fallback);
    let mode = std::fs::read_to_string("/proc/sys/vm/overcommit_memory").ok();
    let commit = match mode.as_deref().map(str::trim) {
        Some("2") => kib(&mem, "CommitLimit:")
            .zip(kib(&mem, "Committed_AS:"))
            .map(|(limit, used)| limit.saturating_sub(used))
            .unwrap_or(fallback),
        Some("0" | "1") => physical,
        _ => fallback,
    };
    Some(MemoryFacts {
        available_physical: physical.min(group),
        available_commit: commit.min(data),
        available_address_space: address,
    })
}
fn kib(text: &str, name: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix(name))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)
}
fn remaining_limit(address: bool, used: u64) -> Option<u64> {
    let resource = if address {
        libc::RLIMIT_AS
    } else {
        libc::RLIMIT_DATA
    };
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit writes the provided live structure for this process.
    if unsafe { libc::getrlimit(resource, &mut limit) } != 0 {
        return None;
    }
    Some(if limit.rlim_cur == libc::RLIM_INFINITY {
        u64::MAX
    } else {
        (limit.rlim_cur as u64).saturating_sub(used)
    })
}
fn unescape(value: &str) -> Option<PathBuf> {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            let oct = bytes.get(i + 1..i + 4)?;
            if !oct.iter().all(|b| (b'0'..=b'7').contains(b)) {
                return None;
            }
            let value = (u16::from(oct[0] - b'0') * 64)
                + (u16::from(oct[1] - b'0') * 8)
                + u16::from(oct[2] - b'0');
            out.push(u8::try_from(value).ok()?);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let path = PathBuf::from(String::from_utf8(out).ok()?);
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return None;
    }
    Some(path)
}
fn membership_path(text: &str, v2: bool) -> Option<&str> {
    text.lines().find_map(|line| {
        let mut fields = line.splitn(3, ':');
        let id = fields.next()?;
        let controllers = fields.next()?;
        let path = fields.next()?;
        ((v2 && id == "0" && controllers.is_empty())
            || (!v2 && controllers.split(',').any(|c| c == "memory")))
        .then_some(path)
    })
}
fn cgroup_headroom() -> Option<u64> {
    cgroup_headroom_at(
        Path::new("/proc/self/cgroup"),
        Path::new("/proc/self/mountinfo"),
    )
}
pub(super) fn cgroup_headroom_at(membership_file: &Path, mounts_file: &Path) -> Option<u64> {
    let membership = std::fs::read_to_string(membership_file).ok()?;
    let mounts = std::fs::read_to_string(mounts_file).ok()?;
    // A v1 memory controller takes precedence on a hybrid hierarchy.
    let v2 = membership_path(&membership, false).is_none();
    let member = unescape(membership_path(&membership, v2)?)?;
    let mut matched = None;
    for line in mounts.lines() {
        let (left, right) = line.split_once(" - ")?;
        let mut right = right.split_whitespace();
        let fs = right.next()?;
        right.next()?;
        let options = right.next()?;
        if if v2 {
            fs != "cgroup2"
        } else {
            fs != "cgroup" || !options.split(',').any(|o| o == "memory")
        } {
            continue;
        }
        let fields: Vec<_> = left.split_whitespace().collect();
        let root = unescape(fields.get(3)?)?;
        let mount = unescape(fields.get(4)?)?;
        if let Ok(relative) = member.strip_prefix(&root) {
            if matched
                .as_ref()
                .is_none_or(|(old, _, _): &(PathBuf, PathBuf, PathBuf)| {
                    root.components().count() > old.components().count()
                })
            {
                matched = Some((root, mount.clone(), mount.join(relative)));
            }
        }
    }
    let (root, mount, leaf) = matched?;
    // A namespace can show a non-root cgroup as "/". Only the absence of
    // both CFTYPE_NOT_ON_ROOT files proves the global v2 root; v1 has no
    // equivalent proof here and therefore retains the conservative ceiling.
    let identity = directory_identity(&mount);
    let root_proven = v2 && root == Path::new("/") && identity.is_some() && global_v2_root(&mount);
    let remaining = hierarchy_headroom(&mount, &leaf, v2)?;
    let root_after = global_v2_root(&mount);
    let stable = std::fs::read_to_string(membership_file).ok().as_deref()
        == Some(membership.as_str())
        && std::fs::read_to_string(mounts_file).ok().as_deref() == Some(mounts.as_str())
        && directory_identity(&mount) == identity;
    let complete = root_proven && root_after && stable;
    Some(remaining.min(if complete {
        u64::MAX
    } else {
        MAX_WORKING_BYTES * 4
    }))
}
fn directory_identity(path: &Path) -> Option<(u64, u64)> {
    let metadata = std::fs::metadata(path).ok()?;
    metadata
        .is_dir()
        .then_some((metadata.dev(), metadata.ino()))
}
fn global_v2_root(mount: &Path) -> bool {
    if std::fs::read_to_string(mount.join("cgroup.controllers")).is_err() {
        return false;
    }
    ["cgroup.events","cgroup.type"].iter().all(|name|
        matches!(std::fs::read_to_string(mount.join(name)),Err(error) if error.raw_os_error()==Some(libc::ENOENT)))
}

fn optional_number(path: &Path) -> Option<Option<u64>> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim() == "max" => Some(Some(u64::MAX)),
        Ok(text) => text.trim().parse().ok().map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(None),
        Err(_) => None,
    }
}
pub(super) fn hierarchy_headroom(mount: &Path, leaf: &Path, v2: bool) -> Option<u64> {
    if !leaf.starts_with(mount) {
        return None;
    }
    let mut cursor = leaf;
    let mut remaining = u64::MAX;
    let observed = (|| {
        loop {
            if !std::fs::metadata(cursor).ok()?.is_dir() {
                return None;
            }
            let (limit, usage) = if v2 {
                ("memory.max", "memory.current")
            } else {
                ("memory.limit_in_bytes", "memory.usage_in_bytes")
            };
            if let Some(limit) = optional_number(&cursor.join(limit))? {
                let used = optional_number(&cursor.join(usage))??;
                remaining = remaining.min(limit.saturating_sub(used));
            } else if v2 {
                // No memory controller file is meaningful only in a live cgroup,
                // not after removal or an unreadable mount/membership transition.
                std::fs::read_to_string(cursor.join("cgroup.controllers")).ok()?;
            } else {
                return None;
            }
            if !std::fs::metadata(cursor).ok()?.is_dir() {
                return None;
            }
            if cursor == mount {
                break;
            }
            cursor = cursor.parent()?;
        }
        Some(remaining)
    })();
    // Keep a known tight descendant bound even if a later ancestor cannot be
    // read. Incomplete ancestry can never authorize the extended class.
    observed.or_else(|| (remaining != u64::MAX).then_some(remaining.min(MAX_WORKING_BYTES * 4)))
}
