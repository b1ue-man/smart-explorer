use std::path::Path;

pub(crate) const MAX_RECLAIM_ERRORS: usize = 64;

pub(crate) fn push_bounded_error(errors: &mut Vec<String>, suppressed: &mut u64, error: String) {
    if errors.len() < MAX_RECLAIM_ERRORS {
        errors.push(error);
    } else {
        *suppressed = suppressed.saturating_add(1);
    }
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(systemtime_ms_from_duration)
        .unwrap_or(0)
}

pub(crate) fn stale_cutoff_ms(now: i64, stale_days: u64) -> i64 {
    let age_ms = stale_days.saturating_mul(86_400_000);
    now.saturating_sub(i64::try_from(age_ms).unwrap_or(i64::MAX))
}

pub(crate) fn systemtime_ms(t: std::time::SystemTime) -> i64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .map(systemtime_ms_from_duration)
        .unwrap_or(0)
}

fn systemtime_ms_from_duration(d: std::time::Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

pub(crate) fn to_fwd(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(crate) fn join_path(parent: &str, name: &str) -> String {
    let p = parent.trim_end_matches('/');
    let n = name.trim_start_matches('/');
    if p.is_empty() || p == "/" {
        format!("/{}", n)
    } else {
        format!("{}/{}", p, n)
    }
}

pub(crate) fn rel_join(parent: &str, name: &str) -> String {
    let p = parent.trim_matches('/');
    let n = name.trim_matches('/');
    if p.is_empty() {
        n.to_string()
    } else if n.is_empty() {
        p.to_string()
    } else {
        format!("{}/{}", p, n)
    }
}

pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{stale_cutoff_ms, systemtime_ms_from_duration};

    #[test]
    fn huge_age_and_duration_saturate_without_wrapping() {
        assert_eq!(
            stale_cutoff_ms(10, u64::MAX),
            10i64.saturating_sub(i64::MAX)
        );
        assert_eq!(
            systemtime_ms_from_duration(std::time::Duration::MAX),
            i64::MAX
        );
    }
}
