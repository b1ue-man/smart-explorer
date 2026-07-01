use super::*;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn temp_dir(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    p.push(format!("vfs_test_{}_{}_{}", tag, std::process::id(), nanos));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn fwd(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

#[test]
fn local_list_and_stat() {
    let dir = temp_dir("list");
    std::fs::write(dir.join("a.txt"), b"hello").unwrap();
    std::fs::create_dir(dir.join("sub")).unwrap();
    let be = LocalBackend::new(&fwd(&dir));
    assert_eq!(be.scheme(), Scheme::Local);
    assert_eq!(be.root_display(), fwd(&dir));

    let mut entries = be.list_dir(&fwd(&dir)).unwrap();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(entries.len(), 2);
    let a = entries.iter().find(|e| e.name == "a.txt").unwrap();
    assert!(!a.is_dir && a.size == 5);
    assert!(entries.iter().find(|e| e.name == "sub").unwrap().is_dir);

    let m = be.stat(&format!("{}/a.txt", fwd(&dir))).unwrap();
    assert_eq!(m.name, "a.txt");
    assert_eq!(m.size, 5);
    assert!(be.exists(&format!("{}/a.txt", fwd(&dir))));
    assert!(!be.exists(&format!("{}/nope", fwd(&dir))));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn local_read_write_copy_rename_remove() {
    let dir = temp_dir("rw");
    let be = LocalBackend::new(&fwd(&dir));
    let nested = format!("{}/x/y", fwd(&dir));
    be.mkdir_all(&nested).unwrap();
    let src = format!("{}/src.bin", fwd(&dir));
    be.open_write(&src)
        .unwrap()
        .write_all(b"0123456789")
        .unwrap();

    let dst = format!("{}/copied.bin", nested);
    assert_eq!(be.copy_file(&src, &dst).unwrap(), 10);
    let mut buf = String::new();
    be.open_read(&dst)
        .unwrap()
        .read_to_string(&mut buf)
        .unwrap();
    assert_eq!(buf, "0123456789");

    let renamed = format!("{}/renamed.bin", nested);
    be.rename(&dst, &renamed).unwrap();
    assert!(!be.exists(&dst) && be.exists(&renamed));

    be.remove_file(&renamed).unwrap();
    be.remove_dir(&nested).unwrap();
    assert!(!be.exists(&nested));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn copy_file_default_impl_streams() {
    // Exercise the trait's default streaming copy_file (not LocalBackend's
    // override) so the remote backends' inherited path is covered.
    struct Streamed(LocalBackend);
    impl Backend for Streamed {
        fn scheme(&self) -> Scheme {
            self.0.scheme()
        }
        fn root_display(&self) -> String {
            self.0.root_display()
        }
        fn list_dir(&self, p: &str) -> VfsResult<Vec<VfsMeta>> {
            self.0.list_dir(p)
        }
        fn stat(&self, p: &str) -> VfsResult<VfsMeta> {
            self.0.stat(p)
        }
        fn open_read(&self, p: &str) -> VfsResult<Box<dyn Read + Send>> {
            self.0.open_read(p)
        }
        fn open_write(&self, p: &str) -> VfsResult<Box<dyn Write + Send>> {
            self.0.open_write(p)
        }
        fn rename(&self, s: &str, d: &str) -> VfsResult<()> {
            self.0.rename(s, d)
        }
        fn remove_file(&self, p: &str) -> VfsResult<()> {
            self.0.remove_file(p)
        }
        fn remove_dir(&self, p: &str) -> VfsResult<()> {
            self.0.remove_dir(p)
        }
        fn mkdir_all(&self, p: &str) -> VfsResult<()> {
            self.0.mkdir_all(p)
        }
    }
    let dir = temp_dir("stream");
    let be = Streamed(LocalBackend::new(&fwd(&dir)));
    let src = format!("{}/s", fwd(&dir));
    be.open_write(&src).unwrap().write_all(b"abcdef").unwrap();
    let dst = format!("{}/d", fwd(&dir));
    assert_eq!(be.copy_file(&src, &dst).unwrap(), 6);
    let mut buf = Vec::new();
    be.open_read(&dst).unwrap().read_to_end(&mut buf).unwrap();
    assert_eq!(buf, b"abcdef");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn caching_backend_serves_and_invalidates() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    // Counts how many times the inner backend is actually hit.
    struct Counter(AtomicUsize);
    impl Backend for Counter {
        fn scheme(&self) -> Scheme {
            Scheme::Sftp
        }
        fn root_display(&self) -> String {
            "/".into()
        }
        fn list_dir(&self, _p: &str) -> VfsResult<Vec<VfsMeta>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Vec::new())
        }
        fn stat(&self, _p: &str) -> VfsResult<VfsMeta> {
            Err(io::Error::from(io::ErrorKind::NotFound))
        }
        fn open_read(&self, _p: &str) -> VfsResult<Box<dyn Read + Send>> {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
        fn open_write(&self, _p: &str) -> VfsResult<Box<dyn Write + Send>> {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
        fn rename(&self, _s: &str, _d: &str) -> VfsResult<()> {
            Ok(())
        }
        fn remove_file(&self, _p: &str) -> VfsResult<()> {
            Ok(())
        }
        fn remove_dir(&self, _p: &str) -> VfsResult<()> {
            Ok(())
        }
        fn mkdir_all(&self, _p: &str) -> VfsResult<()> {
            Ok(())
        }
    }
    // Keep a typed handle so we can read the inner hit-counter directly
    // (the trait object hides it).
    let typed = Arc::new(Counter(AtomicUsize::new(0)));
    let cb2 = CachingBackend::new(typed.clone() as BackendHandle);
    cb2.list_dir("/x").unwrap();
    cb2.list_dir("/x").unwrap();
    cb2.list_dir("/x/").unwrap(); // trailing slash -> same cache key
    assert_eq!(
        typed.0.load(Ordering::SeqCst),
        1,
        "repeat listings served from cache"
    );
    cb2.invalidate_cache();
    cb2.list_dir("/x").unwrap();
    assert_eq!(typed.0.load(Ordering::SeqCst), 2, "refresh re-listed");
    cb2.remove_dir("/x/sub").unwrap(); // invalidates parent "/x"
    cb2.list_dir("/x").unwrap();
    assert_eq!(
        typed.0.load(Ordering::SeqCst),
        3,
        "mutation invalidated the dir"
    );
}

#[test]
fn caching_backend_stat_reuses_fresh_parent_listing() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Counter {
        stat_hits: AtomicUsize,
    }
    impl Backend for Counter {
        fn scheme(&self) -> Scheme {
            Scheme::GDrive
        }
        fn root_display(&self) -> String {
            "/".into()
        }
        fn list_dir(&self, p: &str) -> VfsResult<Vec<VfsMeta>> {
            assert_eq!(p, "/x");
            Ok(vec![VfsMeta {
                name: "a.txt".into(),
                size: 7,
                id: Some("id-a".into()),
                ..Default::default()
            }])
        }
        fn stat(&self, _p: &str) -> VfsResult<VfsMeta> {
            self.stat_hits.fetch_add(1, Ordering::SeqCst);
            Ok(VfsMeta {
                name: "from-stat".into(),
                ..Default::default()
            })
        }
        fn open_read(&self, _p: &str) -> VfsResult<Box<dyn Read + Send>> {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
        fn open_write(&self, _p: &str) -> VfsResult<Box<dyn Write + Send>> {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
        fn rename(&self, _s: &str, _d: &str) -> VfsResult<()> {
            Ok(())
        }
        fn remove_file(&self, _p: &str) -> VfsResult<()> {
            Ok(())
        }
        fn remove_dir(&self, _p: &str) -> VfsResult<()> {
            Ok(())
        }
        fn mkdir_all(&self, _p: &str) -> VfsResult<()> {
            Ok(())
        }
    }

    let inner = Arc::new(Counter {
        stat_hits: AtomicUsize::new(0),
    });
    let cached = CachingBackend::new(inner.clone() as BackendHandle);
    cached.list_dir("/x").unwrap();
    let meta = cached.stat("/x/a.txt").unwrap();
    assert_eq!(meta.name, "a.txt");
    assert_eq!(meta.size, 7);
    assert_eq!(meta.id.as_deref(), Some("id-a"));
    assert_eq!(inner.stat_hits.load(Ordering::SeqCst), 0);
}

#[test]
fn caching_backend_stat_falls_back_for_duplicate_names() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Counter {
        stat_hits: AtomicUsize,
    }
    impl Backend for Counter {
        fn scheme(&self) -> Scheme {
            Scheme::GDrive
        }
        fn root_display(&self) -> String {
            "/".into()
        }
        fn list_dir(&self, _p: &str) -> VfsResult<Vec<VfsMeta>> {
            Ok(vec![
                VfsMeta {
                    name: "dup.txt".into(),
                    id: Some("id-a".into()),
                    ..Default::default()
                },
                VfsMeta {
                    name: "dup.txt".into(),
                    id: Some("id-b".into()),
                    ..Default::default()
                },
            ])
        }
        fn stat(&self, _p: &str) -> VfsResult<VfsMeta> {
            self.stat_hits.fetch_add(1, Ordering::SeqCst);
            Ok(VfsMeta {
                name: "from-stat".into(),
                ..Default::default()
            })
        }
        fn open_read(&self, _p: &str) -> VfsResult<Box<dyn Read + Send>> {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
        fn open_write(&self, _p: &str) -> VfsResult<Box<dyn Write + Send>> {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
        fn rename(&self, _s: &str, _d: &str) -> VfsResult<()> {
            Ok(())
        }
        fn remove_file(&self, _p: &str) -> VfsResult<()> {
            Ok(())
        }
        fn remove_dir(&self, _p: &str) -> VfsResult<()> {
            Ok(())
        }
        fn mkdir_all(&self, _p: &str) -> VfsResult<()> {
            Ok(())
        }
    }

    let inner = Arc::new(Counter {
        stat_hits: AtomicUsize::new(0),
    });
    let cached = CachingBackend::new(inner.clone() as BackendHandle);
    cached.list_dir("/x").unwrap();
    let meta = cached.stat("/x/dup.txt").unwrap();
    assert_eq!(meta.name, "from-stat");
    assert_eq!(inner.stat_hits.load(Ordering::SeqCst), 1);
}

#[test]
fn caching_backend_stat_cache_invalidates_after_mutation() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Counter {
        list_hits: AtomicUsize,
        stat_hits: AtomicUsize,
    }
    impl Backend for Counter {
        fn scheme(&self) -> Scheme {
            Scheme::Sftp
        }
        fn root_display(&self) -> String {
            "/".into()
        }
        fn list_dir(&self, _p: &str) -> VfsResult<Vec<VfsMeta>> {
            self.list_hits.fetch_add(1, Ordering::SeqCst);
            Ok(vec![VfsMeta {
                name: "a.txt".into(),
                ..Default::default()
            }])
        }
        fn stat(&self, _p: &str) -> VfsResult<VfsMeta> {
            self.stat_hits.fetch_add(1, Ordering::SeqCst);
            Ok(VfsMeta {
                name: "from-stat".into(),
                ..Default::default()
            })
        }
        fn open_read(&self, _p: &str) -> VfsResult<Box<dyn Read + Send>> {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
        fn open_write(&self, _p: &str) -> VfsResult<Box<dyn Write + Send>> {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
        fn rename(&self, _s: &str, _d: &str) -> VfsResult<()> {
            Ok(())
        }
        fn remove_file(&self, _p: &str) -> VfsResult<()> {
            Ok(())
        }
        fn remove_dir(&self, _p: &str) -> VfsResult<()> {
            Ok(())
        }
        fn mkdir_all(&self, _p: &str) -> VfsResult<()> {
            Ok(())
        }
    }

    let inner = Arc::new(Counter {
        list_hits: AtomicUsize::new(0),
        stat_hits: AtomicUsize::new(0),
    });
    let cached = CachingBackend::new(inner.clone() as BackendHandle);
    cached.list_dir("/x").unwrap();
    assert_eq!(cached.stat("/x/a.txt").unwrap().name, "a.txt");
    cached.remove_file("/x/a.txt").unwrap();
    assert_eq!(cached.stat("/x/a.txt").unwrap().name, "from-stat");
    assert_eq!(inner.list_hits.load(Ordering::SeqCst), 1);
    assert_eq!(inner.stat_hits.load(Ordering::SeqCst), 1);
}

#[test]
fn dispatch_and_remote_detection() {
    assert_eq!(backend_for("/tmp").unwrap().scheme(), Scheme::Local);
    assert_eq!(backend_for(r"C:\Users").unwrap().scheme(), Scheme::Local);
    assert_eq!(
        backend_for(r"\\server\share").unwrap().scheme(),
        Scheme::Local
    );
    assert!(backend_for("sftp://h/p").is_err());
    assert!(backend_for("ftp://h/p").is_err());
    assert!(backend_for("ftps://h/p").is_err());

    assert!(!is_remote_root(r"C:\Users"));
    assert!(!is_remote_root(r"\\server\share"));
    assert!(is_remote_root("sftp://h/p"));
    assert!(is_remote_root("FTP://H/P"));
}
