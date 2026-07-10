use super::*;
use crate::vfs::{LocalBackend, Scheme, VfsMeta, VfsResult};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "bisync_apply_{tag}_{}_{}",
        std::process::id(),
        nanos
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn forward(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn conflict_siblings_never_replace_same_second_names() {
    let root_path = temp_dir("conflict_unique");
    let root = forward(&root_path);
    let backend = LocalBackend::new(&root);
    let source = format!("{root}/f.txt");
    let first = format!("{root}/f (Konflikt 20260101-010203).txt");
    std::fs::write(&source, b"loser").unwrap();
    std::fs::write(&first, b"preexisting").unwrap();
    let throttle = Throttle::new(0);
    let cancel = AtomicBool::new(false);

    let (_, second) = copy_conflict_sibling_at(
        &backend,
        &source,
        &root,
        "f.txt",
        ExpectedFile::Unknown,
        &throttle,
        &cancel,
        "20260101-010203",
    )
    .unwrap();
    let (_, third) = copy_conflict_sibling_at(
        &backend,
        &source,
        &root,
        "f.txt",
        ExpectedFile::Unknown,
        &throttle,
        &cancel,
        "20260101-010203",
    )
    .unwrap();

    assert_eq!(std::fs::read(first).unwrap(), b"preexisting");
    assert!(second.ends_with("f (Konflikt 20260101-010203 2).txt"));
    assert!(third.ends_with("f (Konflikt 20260101-010203 3).txt"));
    assert_eq!(std::fs::read(second).unwrap(), b"loser");
    assert_eq!(std::fs::read(third).unwrap(), b"loser");
    std::fs::remove_dir_all(root_path).ok();
}

enum Hook {
    DriftDestination {
        target: String,
        stats: AtomicUsize,
    },
    #[cfg(unix)]
    SwapSourceToLink {
        source: String,
        link_target: String,
        swapped: AtomicBool,
    },
}

struct HookBackend {
    inner: LocalBackend,
    hook: Hook,
}

impl Backend for HookBackend {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }

    fn root_display(&self) -> String {
        self.inner.root_display()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.inner.list_dir(path)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        match &self.hook {
            Hook::DriftDestination { target, stats } => {
                if path == target && stats.fetch_add(1, Ordering::Relaxed) == 3 {
                    std::fs::write(target, b"concurrent-destination-change")?;
                }
            }
            #[cfg(unix)]
            Hook::SwapSourceToLink { .. } => {}
        }
        self.inner.stat(path)
    }

    fn try_exists(&self, path: &str) -> VfsResult<bool> {
        self.inner.try_exists(path)
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let reader = self.inner.open_read(path)?;
        #[cfg(unix)]
        if let Hook::SwapSourceToLink {
            source,
            link_target,
            swapped,
        } = &self.hook
        {
            if path == source && !swapped.swap(true, Ordering::Relaxed) {
                std::fs::remove_file(source)?;
                std::os::unix::fs::symlink(link_target, source)?;
            }
        }
        Ok(reader)
    }

    fn open_read_id(&self, path: &str, _id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        self.open_read(path)
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(path)
    }

    fn rename(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.inner.rename(source, destination)
    }

    fn rename_no_replace(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.inner.rename_no_replace(source, destination)
    }

    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.inner.promote_staged(staged, destination)
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_file(path)
    }

    fn remove_file_id(&self, path: &str, id: Option<&str>) -> VfsResult<()> {
        self.inner.remove_file_id(path, id)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_dir(path)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.inner.mkdir_all(path)
    }

    fn is_local(&self) -> bool {
        true
    }

    fn rename_overwrites(&self) -> bool {
        self.inner.rename_overwrites()
    }
}

#[test]
fn destination_drift_after_backup_blocks_promotion() {
    let source_root_path = temp_dir("drift_source");
    let destination_root_path = temp_dir("drift_destination");
    let versions = temp_dir("drift_versions");
    let source_root = forward(&source_root_path);
    let destination_root = forward(&destination_root_path);
    let source_path = format!("{source_root}/f.txt");
    let destination_path = format!("{destination_root}/f.txt");
    std::fs::write(&source_path, b"new").unwrap();
    std::fs::write(&destination_path, b"old").unwrap();
    let source = LocalBackend::new(&source_root);
    let destination = HookBackend {
        inner: LocalBackend::new(&destination_root),
        hook: Hook::DriftDestination {
            target: destination_path.clone(),
            stats: AtomicUsize::new(0),
        },
    };
    let cancel = AtomicBool::new(false);
    let throttle = Throttle::new(0);

    let error = copy_replace(
        &source,
        &source_path,
        ExpectedFile::Unknown,
        &destination,
        &destination_path,
        ExpectedFile::Unknown,
        Some(("f.txt", &versions)),
        &throttle,
        &cancel,
    )
    .unwrap_err()
    .into_io();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        std::fs::read(&destination_path).unwrap(),
        b"concurrent-destination-change"
    );
    for path in [source_root_path, destination_root_path, versions] {
        std::fs::remove_dir_all(path).ok();
    }
}

#[test]
fn source_drift_from_planned_signature_blocks_copy() {
    let source_root_path = temp_dir("planned_source");
    let destination_root_path = temp_dir("planned_destination");
    let source_root = forward(&source_root_path);
    let destination_root = forward(&destination_root_path);
    let source_path = format!("{source_root}/f.txt");
    let destination_path = format!("{destination_root}/f.txt");
    std::fs::write(&source_path, b"changed-after-plan").unwrap();
    let source = LocalBackend::new(&source_root);
    let destination = LocalBackend::new(&destination_root);
    let cancel = AtomicBool::new(false);
    let throttle = Throttle::new(0);

    let error = copy_replace(
        &source,
        &source_path,
        ExpectedFile::Present(super::super::types::Sig {
            size: 3,
            mtime_ms: 0,
            hash: 0,
        }),
        &destination,
        &destination_path,
        ExpectedFile::Missing,
        None,
        &throttle,
        &cancel,
    )
    .unwrap_err()
    .into_io();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(!destination_root_path.join("f.txt").exists());
    for path in [source_root_path, destination_root_path] {
        std::fs::remove_dir_all(path).ok();
    }
}

#[cfg(unix)]
#[test]
fn source_link_swap_after_open_blocks_promotion() {
    let source_root_path = temp_dir("link_source");
    let destination_root_path = temp_dir("link_destination");
    let source_root = forward(&source_root_path);
    let destination_root = forward(&destination_root_path);
    let source_path = format!("{source_root}/f.txt");
    let link_target = format!("{source_root}/outside.txt");
    let destination_path = format!("{destination_root}/f.txt");
    std::fs::write(&source_path, b"planned").unwrap();
    std::fs::write(&link_target, b"outside").unwrap();
    let source = HookBackend {
        inner: LocalBackend::new(&source_root),
        hook: Hook::SwapSourceToLink {
            source: source_path.clone(),
            link_target,
            swapped: AtomicBool::new(false),
        },
    };
    let destination = LocalBackend::new(&destination_root);
    let cancel = AtomicBool::new(false);
    let throttle = Throttle::new(0);

    let error = copy_replace(
        &source,
        &source_path,
        ExpectedFile::Unknown,
        &destination,
        &destination_path,
        ExpectedFile::Missing,
        None,
        &throttle,
        &cancel,
    )
    .unwrap_err()
    .into_io();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(!destination_root_path.join("f.txt").exists());
    for path in [source_root_path, destination_root_path] {
        std::fs::remove_dir_all(path).ok();
    }
}
