#[inline]
pub(super) fn ms_since_unix(t: std::time::SystemTime) -> i64 {
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

#[inline]
pub(super) fn ext_of(name: &str, is_dir: bool) -> String {
    if is_dir {
        return String::new();
    }
    match name.rfind('.') {
        Some(i) if i + 1 < name.len() && i > 0 => name[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}
