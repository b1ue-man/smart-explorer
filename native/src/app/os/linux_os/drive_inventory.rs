//! Process mount namespace inventory, including the no-UDisks2 fallback.
use crate::app::drive_snapshot::DriveEntry;
use std::collections::BTreeMap;
use std::ffi::CString;

pub(crate) fn read_drive_inventory() -> Result<Vec<DriveEntry>, String> {
    let mounts = std::fs::read_to_string("/proc/self/mountinfo").map_err(|e| e.to_string())?;
    let uuids = device_uuids();
    let mut entries = BTreeMap::new();
    for line in mounts.lines() {
        let Some((before, after)) = line.split_once(" - ") else {
            continue;
        };
        let fields: Vec<_> = before.split_whitespace().collect();
        let fs: Vec<_> = after.split_whitespace().collect();
        if fields.len() < 6 || fs.len() < 2 {
            continue;
        }
        let Some(root) = unescape(fields[4]) else {
            continue;
        };
        let Some(source) = unescape(fs[1]) else {
            continue;
        };
        // Exclude kernel API mounts; retain root, block, network and FUSE storage.
        if root != "/"
            && !(source.starts_with("/dev/")
                || fs[0].starts_with("fuse")
                || matches!(fs[0], "nfs" | "nfs4" | "cifs" | "smb3" | "9p"))
        {
            continue;
        }
        let Some(subroot) = unescape(fields[3]) else {
            continue;
        };
        let identity = if source.starts_with("/dev/") {
            let canonical = std::fs::canonicalize(&source)
                .ok()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|| source.clone());
            uuids
                .get(&canonical)
                .map(|uuid| format!("uuid:{uuid}:{subroot}"))
                .or_else(|| {
                    disk_sequence(fields[2])
                        .map(|sequence| format!("block:{}:{sequence}:{subroot}", fields[2]))
                })
        } else {
            // A FUSE/network source label proves neither the principal nor
            // the root object. It cannot authorize automatic reconnection.
            None
        };
        // mountinfo IDs are valid only for the current mount namespace and
        // may be reused after unmount. Keep this process-local attachment
        // marker separate from the persistent media identity above.
        let incarnation = Some(format!(
            "{}:{}:{}:{source}:{subroot}",
            fields[0], fields[2], fs[0]
        ));
        entries.insert(
            root.clone(),
            DriveEntry {
                root,
                identity,
                incarnation,
                free: 0,
                total: 0,
            },
        );
    }
    if entries.is_empty() {
        return Err("Mountbestand enthält keine Speicherwurzel".into());
    }
    Ok(entries.into_values().collect())
}

// Linux diskseq distinguishes a replacement reusing the same major/minor.
// A partition inherits its enclosing disk's sequence. Without either UUID or
// sequence, identity stays unknown rather than treating /dev/sdX as proof.
fn disk_sequence(device: &str) -> Option<String> {
    let path = std::fs::canonicalize(format!("/sys/dev/block/{device}")).ok()?;
    for parent in path.ancestors().take(3) {
        if let Ok(sequence) = std::fs::read_to_string(parent.join("diskseq")) {
            let sequence = sequence.trim();
            if !sequence.is_empty() && sequence.bytes().all(|byte| byte.is_ascii_digit()) {
                return Some(sequence.to_string());
            }
        }
    }
    None
}

pub(crate) fn read_drive_capacity(root: &str) -> (u64, u64) {
    let Ok(path) = CString::new(root) else {
        return (0, 0);
    };
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return (0, 0);
    }
    let stats = unsafe { stats.assume_init() };
    (
        (stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64),
        (stats.f_blocks as u64).saturating_mul(stats.f_frsize as u64),
    )
}

fn device_uuids() -> BTreeMap<String, String> {
    let mut uuids = BTreeMap::new();
    if let Ok(entries) = std::fs::read_dir("/dev/disk/by-uuid") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if let (Ok(device), Some(uuid)) = (std::fs::canonicalize(entry.path()), name.to_str()) {
                uuids.insert(device.to_string_lossy().into_owned(), uuid.to_string());
            }
        }
    }
    uuids
}

fn unescape(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            let digits = bytes.get(i + 1..i + 4)?;
            if !digits.iter().all(|d| (b'0'..=b'7').contains(d)) {
                return None;
            }
            let value = (digits[0] - b'0') as u16 * 64
                + (digits[1] - b'0') as u16 * 8
                + (digits[2] - b'0') as u16;
            result.push(u8::try_from(value).ok()?);
            i += 4;
        } else {
            result.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(result).ok()
}
