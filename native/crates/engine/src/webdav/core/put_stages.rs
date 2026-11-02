//! Copy stages whose `PUT` this process saw committed (G3-12, 2026-10-28).
//! The `PUT` carried `If-None-Match: *` and the exact `Content-Length` and
//! was answered 201: the server created exactly this regular resource for
//! this client. Its publication (`vfs::promote_copy_stage_evidence`) then
//! sends the `MOVE` with `Overwrite: F` right away instead of a `PROPFIND`
//! of the stage first; a stage this process did not see committed is
//! published as before. Kept by URL until published, discarded or (beyond
//! `MAX_KEPT`) forgotten, which only costs that `PROPFIND` again.
use std::collections::HashSet;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

const MAX_KEPT: usize = 4096;

fn kept() -> MutexGuard<'static, HashSet<String>> {
    static KEPT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    KEPT.get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// The `PUT` of the stage at `url` was committed.
pub(super) fn committed(url: &str) {
    let mut kept = kept();
    if kept.len() >= MAX_KEPT {
        kept.clear();
    }
    kept.insert(url.to_string());
}

/// Takes the stage at `url`: whether its `PUT` was seen committed.
pub(super) fn take(url: &str) -> bool {
    kept().remove(url)
}

#[cfg(test)]
mod tests {
    #[test]
    fn speed_task_webdav_put_stages_are_taken_once() {
        let url = "http://dav.test/dir/a.se-upload-0123456789abcdef";
        super::committed(url);
        assert!(super::take(url));
        assert!(!super::take(url), "taken once");
    }
}
