use super::{Progress, ScanOutcome, ScanPhase};
use crate::agent_proto::WireNode;
use crate::vfs::{Backend, Scheme};
use std::sync::atomic::Ordering;

/// The same remote selection boundary is used by the GUI, the Android app
/// and exporting workers: the device that holds the data analyses it
/// (`scan_storage`), an older peer or an SSH agent walks it on its side
/// (`walk_tree`), and only a backend without either is listed from here.
pub fn scan_remote(backend: &dyn Backend, root: &str, progress: &Progress) -> ScanOutcome {
    let segment = progress.remote_segment();
    let progress = &segment;
    let before = progress.snapshot();
    progress.set_phase(ScanPhase::Preparing, root);
    let result = backend.scan_storage(root, progress);
    match result {
        Ok(Some(outcome)) => return outcome,
        Err(_) if progress.check_cancel().is_err() => return ScanOutcome::canceled(),
        Err(error) => return ScanOutcome::failed(root, error.to_string()),
        Ok(None) => {}
    }
    if !backend.supports_walk_tree() {
        return super::scan_backend(backend, root, progress);
    }
    progress.set_phase(ScanPhase::Legacy, root);
    let on_progress = |files, bytes| {
        progress
            .files
            .store(before.files.saturating_add(files), Ordering::Relaxed);
        progress
            .bytes
            .store(before.bytes.saturating_add(bytes), Ordering::Relaxed);
        progress.check_cancel().is_ok()
    };
    match backend.walk_tree(root, &on_progress) {
        Ok(Some(tree)) if progress.check_cancel().is_ok() => {
            // The last progress report may lag behind the finished walk; the
            // counters must describe exactly the tree that is returned.
            let (files, bytes) = (file_count(&tree), tree.size);
            progress
                .files
                .store(before.files.saturating_add(files), Ordering::Relaxed);
            progress
                .bytes
                .store(before.bytes.saturating_add(bytes), Ordering::Relaxed);
            let mut outcome = ScanOutcome::complete(super::from_wire(tree));
            // Only an older Smart Explorer peer has a newer path to offer; an
            // SSH agent always walks this way and reports no folder counts.
            if progress.snapshot().phase == ScanPhase::Legacy && backend.scheme() == Scheme::Peer {
                outcome.notes.push("Die Gegenstelle nutzt den älteren Analysepfad; für den lokalen Worker und vollständige Fortschrittsmeldungen beide Geräte aktualisieren.".into());
            }
            outcome
        }
        _ if progress.check_cancel().is_err() => ScanOutcome::canceled(),
        Ok(None) => super::scan_backend(backend, root, progress),
        Err(error) => ScanOutcome::failed(root, error.to_string()),
        Ok(Some(_)) => ScanOutcome::canceled(),
    }
}

/// Files of a walked tree; one iterator per level, never recursion.
fn file_count(tree: &WireNode) -> u64 {
    if !tree.is_dir {
        return 1;
    }
    let mut files = 0u64;
    let mut levels = vec![tree.children.iter()];
    while let Some(children) = levels.last_mut() {
        match children.next() {
            Some(child) if child.is_dir => levels.push(child.children.iter()),
            Some(_) => files = files.saturating_add(1),
            None => {
                levels.pop();
            }
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::{VfsMeta, VfsResult};
    use std::io::{self, Read, Write};

    /// An SSH agent: no storage worker, a server-side walk whose last
    /// progress report lags behind the finished tree.
    struct AgentWalk(Scheme);

    impl Backend for AgentWalk {
        fn scheme(&self) -> Scheme {
            self.0
        }
        fn root_display(&self) -> String {
            "/".into()
        }
        fn supports_walk_tree(&self) -> bool {
            true
        }
        fn walk_tree(
            &self,
            _root: &str,
            on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
        ) -> VfsResult<Option<WireNode>> {
            on_progress(1, 10);
            let file = |name: &str, size| WireNode {
                name: name.into(),
                size,
                is_dir: false,
                children: Vec::new(),
            };
            Ok(Some(WireNode {
                name: "root".into(),
                size: 30,
                is_dir: true,
                children: vec![
                    file("a", 10),
                    WireNode {
                        name: "sub".into(),
                        size: 20,
                        is_dir: true,
                        children: vec![file("b", 15), file("c", 5)],
                    },
                ],
            }))
        }
        fn list_dir(&self, _path: &str) -> VfsResult<Vec<VfsMeta>> {
            Err(io::Error::other("not listed"))
        }
        fn stat(&self, _path: &str) -> VfsResult<VfsMeta> {
            Err(io::Error::other("unused"))
        }
        fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
            Err(io::Error::other("unused"))
        }
        fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
            Err(io::Error::other("unused"))
        }
        fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
            Err(io::Error::other("unused"))
        }
        fn remove_file(&self, _path: &str) -> VfsResult<()> {
            Err(io::Error::other("unused"))
        }
        fn remove_dir(&self, _path: &str) -> VfsResult<()> {
            Err(io::Error::other("unused"))
        }
        fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
            Err(io::Error::other("unused"))
        }
    }

    #[test]
    fn review_task_legacy_walk_counts_the_tree_and_names_only_old_peers() {
        let progress = Progress::default();
        let outcome = scan_remote(&AgentWalk(Scheme::Sftp), "/", &progress);
        assert_eq!(outcome.tree.as_ref().map(|tree| tree.size), Some(30));
        assert_eq!(progress.files.load(Ordering::Relaxed), 3);
        assert_eq!(progress.bytes.load(Ordering::Relaxed), 30);
        assert!(outcome.notes.is_empty(), "{:?}", outcome.notes);

        let outcome = scan_remote(&AgentWalk(Scheme::Peer), "/", &Progress::default());
        assert_eq!(outcome.notes.len(), 1);
        assert!(outcome.notes[0].contains("älteren Analysepfad"));
    }
}
