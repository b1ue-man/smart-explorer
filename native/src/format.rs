use crate::types::{FileEntry, SortDir, SortKey};
use chrono::{Local, TimeZone};
use std::cmp::Ordering;

pub fn format_bytes(n: u64) -> String {
    if n < 1024 {
        return format!("{} B", n);
    }
    let units = ["KB", "MB", "GB", "TB", "PB"];
    let mut v = n as f64 / 1024.0;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if v >= 100.0 {
        format!("{:.0} {}", v, units[i])
    } else if v >= 10.0 {
        format!("{:.1} {}", v, units[i])
    } else {
        format!("{:.2} {}", v, units[i])
    }
}

pub fn format_date(ms: i64) -> String {
    if ms == 0 {
        return String::new();
    }
    match Local.timestamp_millis_opt(ms) {
        chrono::LocalResult::Single(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        chrono::LocalResult::Ambiguous(a, _) => a.format("%Y-%m-%d %H:%M").to_string(),
        _ => String::new(),
    }
}

pub fn compare_entries(
    a: &FileEntry,
    b: &FileEntry,
    key: SortKey,
    dir: SortDir,
    dirs_first: bool,
) -> Ordering {
    // Optionally pin directories above files; otherwise both are ranked purely
    // by the active key (so e.g. sorting by date interleaves files and folders).
    if dirs_first && a.is_dir != b.is_dir {
        return if a.is_dir { Ordering::Less } else { Ordering::Greater };
    }
    let cmp = match key {
        SortKey::Name => natural_compare(a.name.as_ref(), b.name.as_ref()),
        SortKey::Path => natural_compare(a.path.as_ref(), b.path.as_ref()),
        SortKey::Size => a.size.cmp(&b.size),
        SortKey::Mtime => a.mtime_ms.cmp(&b.mtime_ms),
        SortKey::Btime => a.btime_ms.cmp(&b.btime_ms),
        SortKey::Ext => a.ext.cmp(&b.ext).then_with(|| natural_compare(a.name.as_ref(), b.name.as_ref())),
        SortKey::Depth => a.depth.cmp(&b.depth),
    };
    match dir {
        SortDir::Asc => cmp,
        SortDir::Desc => cmp.reverse(),
    }
}

/// Natural ordering: "file2" < "file10". Case-insensitive ASCII.
fn natural_compare(a: &str, b: &str) -> Ordering {
    let mut ai = a.bytes();
    let mut bi = b.bytes();
    let (mut a_buf, mut b_buf) = (ai.next(), bi.next());
    while let (Some(ca), Some(cb)) = (a_buf, b_buf) {
        if ca.is_ascii_digit() && cb.is_ascii_digit() {
            // Compare full numeric runs
            let mut na: u64 = (ca - b'0') as u64;
            let mut nb: u64 = (cb - b'0') as u64;
            a_buf = ai.next();
            b_buf = bi.next();
            while let Some(ch) = a_buf {
                if ch.is_ascii_digit() {
                    na = na.saturating_mul(10).saturating_add((ch - b'0') as u64);
                    a_buf = ai.next();
                } else {
                    break;
                }
            }
            while let Some(ch) = b_buf {
                if ch.is_ascii_digit() {
                    nb = nb.saturating_mul(10).saturating_add((ch - b'0') as u64);
                    b_buf = bi.next();
                } else {
                    break;
                }
            }
            match na.cmp(&nb) {
                Ordering::Equal => {}
                ord => return ord,
            }
        } else {
            let la = ca.to_ascii_lowercase();
            let lb = cb.to_ascii_lowercase();
            match la.cmp(&lb) {
                Ordering::Equal => {}
                ord => return ord,
            }
            a_buf = ai.next();
            b_buf = bi.next();
        }
    }
    match (a_buf, b_buf) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        // Loop exits before this happens; treat as equal defensively.
        (Some(_), Some(_)) => Ordering::Equal,
    }
}
