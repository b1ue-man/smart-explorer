//! Path keys of the listing cache: normalized directory keys and parents.
use super::CachingBackend;

impl CachingBackend {
    pub(super) fn norm(path: &str) -> String {
        let p = path.trim_end_matches('/');
        if p.is_empty() {
            "/".to_string()
        } else {
            p.to_string()
        }
    }

    pub(super) fn parent_of(key: &str) -> Option<String> {
        if key == "/" {
            return None;
        }
        key.rfind('/').map(|i| {
            if i == 0 {
                "/".to_string()
            } else {
                key[..i].to_string()
            }
        })
    }

    pub(super) fn parent_and_name(key: &str) -> Option<(String, &str)> {
        if key.is_empty() || key == "/" {
            return None;
        }
        match key.rsplit_once('/') {
            Some((parent, name)) if !name.is_empty() => Some((
                if parent.is_empty() {
                    "/".to_string()
                } else {
                    parent.to_string()
                },
                name,
            )),
            None => Some(("/".to_string(), key)),
            _ => None,
        }
    }
}
