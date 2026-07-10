use super::ops;
use super::target::Target;
use crate::agent_proto::SearchSpec;
use crate::vfs::{Backend, Scheme, SearchHit, VfsMeta, VfsResult};
use std::io::{self, Cursor, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Default)]
struct SearchFailureBackend {
    lists: AtomicUsize,
}

impl Backend for SearchFailureBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn list_dir(&self, _path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.lists.fetch_add(1, Ordering::Relaxed);
        Ok(Vec::new())
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        Err(io::Error::new(io::ErrorKind::NotFound, path))
    }

    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
        Ok(Box::new(Cursor::new(Vec::<u8>::new())))
    }

    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(Vec::<u8>::new()))
    }

    fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
        Ok(())
    }

    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }

    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }

    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }

    fn supports_search(&self) -> bool {
        true
    }

    fn search(
        &self,
        _root: &str,
        _spec: &SearchSpec,
        tx: crossbeam_channel::Sender<SearchHit>,
        _cancel: &AtomicBool,
    ) -> VfsResult<bool> {
        tx.send(SearchHit {
            rel: "partial.txt".into(),
            is_dir: false,
            size: 7,
            mtime_ms: 1,
        })
        .unwrap();
        Err(io::Error::other("late server-side search failure"))
    }
}

#[test]
fn partial_server_search_error_does_not_run_listing_fallback() {
    let backend = Arc::new(SearchFailureBackend::default());
    let target = Target::with_backend_key(backend.clone(), "/".into(), "search-error");

    let error = ops::search(&target, "", false, 0, false).unwrap_err();
    assert!(error.contains("late server-side search failure"));
    assert_eq!(backend.lists.load(Ordering::Relaxed), 0);
}
