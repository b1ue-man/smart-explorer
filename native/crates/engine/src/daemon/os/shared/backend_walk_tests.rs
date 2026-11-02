use super::{handle_search_backend, handle_walk_hashed_backend, handle_walk_tree_backend};
use crate::agent_proto::SearchSpec;
use crate::daemon::backend_server::Sink;
use crate::vfs::{Backend, BackendHandle, Scheme, VfsMeta, VfsResult};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

struct ListingFailure {
    removed: Arc<AtomicBool>,
}

impl Backend for ListingFailure {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn list_dir(&self, _path: &str) -> VfsResult<Vec<VfsMeta>> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
    }
    fn stat(&self, _path: &str) -> VfsResult<VfsMeta> {
        Ok(VfsMeta {
            is_dir: true,
            ..VfsMeta::default()
        })
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
        self.removed.store(true, Ordering::Relaxed);
        Ok(())
    }
    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Err(io::Error::other("unused"))
    }
}

#[test]
fn walks_searches_and_hashes_report_listing_failures() {
    let backend: BackendHandle = Arc::new(ListingFailure {
        removed: Arc::new(AtomicBool::new(false)),
    });
    let sink: Sink = Arc::new(Mutex::new(Box::new(Vec::<u8>::new())));
    let cancel = AtomicBool::new(false);
    let spec = SearchSpec {
        query: String::new(),
        glob: false,
        min_size: 0,
        max_size: 0,
        max_results: 0,
        want_dirs: true,
    };

    assert_eq!(
        handle_walk_tree_backend(&sink, 1, &backend, "/root", &cancel)
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        handle_search_backend(&sink, 2, &backend, "/root", &spec, &cancel)
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        handle_walk_hashed_backend(&sink, 3, &backend, "/root", true, &cancel)
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
}
