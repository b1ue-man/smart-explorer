use super::*;
use crate::vfs::{Backend, Scheme, VfsResult};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
enum Fault {
    UnsafeName,
    DuplicateName,
    DescendantListing,
}

struct FaultBackend {
    fault: Fault,
    removals: Arc<Mutex<Vec<String>>>,
}

impl Backend for FaultBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        if matches!(self.fault, Fault::DescendantListing) && path == "/root/child" {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"));
        }
        if path != "/root" {
            return Ok(Vec::new());
        }
        let name = match self.fault {
            Fault::UnsafeName => "../outside",
            Fault::DuplicateName | Fault::DescendantListing => "child",
        };
        let count = if matches!(self.fault, Fault::DuplicateName) {
            2
        } else {
            1
        };
        Ok((0..count)
            .map(|_| VfsMeta {
                name: name.into(),
                is_dir: !matches!(self.fault, Fault::UnsafeName),
                ..VfsMeta::default()
            })
            .collect())
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        Ok(VfsMeta {
            name: path.rsplit('/').next().unwrap_or(path).into(),
            is_dir: path == "/root" || path == "/root/child",
            ..VfsMeta::default()
        })
    }

    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
        Err(io::Error::other("unused"))
    }

    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Err(io::Error::other("unused"))
    }

    fn rename(&self, _source: &str, _destination: &str) -> VfsResult<()> {
        Err(io::Error::other("unused"))
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.removals.lock().unwrap().push(path.into());
        Ok(())
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.removals.lock().unwrap().push(path.into());
        Ok(())
    }

    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Err(io::Error::other("unused"))
    }
}

#[test]
fn every_planning_fault_happens_before_any_removal() {
    for fault in [
        Fault::UnsafeName,
        Fault::DuplicateName,
        Fault::DescendantListing,
    ] {
        let removals = Arc::new(Mutex::new(Vec::new()));
        let backend: BackendHandle = Arc::new(FaultBackend {
            fault,
            removals: removals.clone(),
        });
        assert!(remove_tree_backend(&backend, "/root", &AtomicBool::new(false)).is_err());
        assert!(removals.lock().unwrap().is_empty());
    }
}

#[test]
fn cancellation_happens_before_any_removal() {
    let removals = Arc::new(Mutex::new(Vec::new()));
    let backend: BackendHandle = Arc::new(FaultBackend {
        fault: Fault::DescendantListing,
        removals: removals.clone(),
    });
    let error = remove_tree_backend(&backend, "/root", &AtomicBool::new(true)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert!(removals.lock().unwrap().is_empty());
}
