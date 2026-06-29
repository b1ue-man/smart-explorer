use std::path::Path;

pub(crate) fn local_scan_threads() -> usize {
    std::env::var("SMART_EXPLORER_ANALYTICS_THREADS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(2)
        .clamp(1, 4)
}

pub(crate) fn truncate<T>(v: &mut Vec<T>, max: usize) {
    if v.len() > max {
        v.truncate(max);
    }
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(systemtime_ms_from_duration)
        .unwrap_or(0)
}

pub(crate) fn systemtime_ms(t: std::time::SystemTime) -> i64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .map(systemtime_ms_from_duration)
        .unwrap_or(0)
}

fn systemtime_ms_from_duration(d: std::time::Duration) -> i64 {
    d.as_secs() as i64 * 1000 + i64::from(d.subsec_millis())
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
