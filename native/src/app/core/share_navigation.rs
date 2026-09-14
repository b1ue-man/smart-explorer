//! Opening a Share peer at a specific folder (favourites, landing tiles).
use super::*;

impl App {
    /// Like `open_share_target`, but lands in `path` once the peer session is
    /// ready. An already open session navigates immediately.
    pub(in crate::app) fn open_share_target_at(
        &mut self,
        target: crate::share::PeerOpenTarget,
        path: Option<String>,
    ) {
        let path = path
            .map(|path| path.trim().replace('\\', "/"))
            .filter(|path| !path.is_empty())
            .map(|path| {
                if path.starts_with('/') {
                    path
                } else {
                    format!("/{path}")
                }
            });
        if self.share_target_is_open(&target) {
            if let Some(path) = path {
                self.start_scan(PathBuf::from(path));
            } else {
                self.notice = Some((
                    "Share-Verbindung ist bereits offen".to_string(),
                    std::time::Instant::now(),
                ));
            }
            return;
        }
        self.share_opening_path = path;
        self.open_share_target(target);
        if self.share_opening.is_none() {
            // The open did not start (policy or spawn failure); drop the path
            // so an unrelated later open does not inherit it.
            self.share_opening_path = None;
        }
    }
}
