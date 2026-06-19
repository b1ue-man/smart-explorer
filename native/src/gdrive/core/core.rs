pub(super) fn norm(path: &str) -> String {
    path.trim().trim_matches('/').to_string()
}

pub(super) fn split_parent(key: &str) -> (String, &str) {
    match key.rsplit_once('/') {
        Some((par, name)) => (par.to_string(), name),
        None => (String::new(), key),
    }
}

pub(super) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Minimal URL-component encoder (reuses the same rules as cloud.rs).
pub(super) fn cloud_urlenc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// RFC 3339 (e.g. "2024-06-01T12:34:56.000Z") -> unix millis (best effort).
pub(super) fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_and_split() {
        assert_eq!(norm("/a/b/"), "a/b");
        assert_eq!(norm("/"), "");
        let (p, n) = split_parent("a/b/c");
        assert_eq!(p, "a/b");
        assert_eq!(n, "c");
        let (p, n) = split_parent("x");
        assert_eq!(p, "");
        assert_eq!(n, "x");
    }

    #[test]
    fn rfc3339_parses() {
        assert!(parse_rfc3339_ms("2024-06-01T12:34:56Z").unwrap() > 0);
        assert!(parse_rfc3339_ms("not a date").is_none());
    }
}
