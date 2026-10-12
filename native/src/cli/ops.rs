use super::entry_kind::EntryKind;
use super::link_paths::{
    copy_remote_link, link_move_refused, resolve_destination, source_root, SourceRoot,
};
use super::target::Target;
use super::transfer::{transfer_plan, transfer_plan_as, try_rename_fast};
use super::tree_observer::{TreeObserver, TreePhase};
use super::tree_ops::{
    copy_entry_with, remove_copied_source, remove_existing, TreeOutcome, TreeRequest,
};
use std::io::{self, Write};
use std::path::PathBuf;

pub(crate) fn list(target: &Target) -> Result<(), String> {
    for entry in target
        .backend
        .list_dir(&target.path)
        .map_err(|e| e.to_string())?
    {
        // Second column: `l` for links (kept for scripts that read it).
        let kind = match EntryKind::of(&entry) {
            EntryKind::Dir => 'd',
            EntryKind::Special => 's',
            EntryKind::File | EntryKind::Link => '-',
        };
        let symlink = if entry.is_symlink { "l" } else { "-" };
        println!("{kind}{symlink}\t{}\t{}", entry.size, entry.name);
    }
    Ok(())
}

pub(crate) fn stat(target: &Target) -> Result<(), String> {
    let meta = target
        .backend
        .stat(&target.path)
        .map_err(|e| e.to_string())?;
    println!("path\t{}", target.path);
    println!("name\t{}", meta.name);
    println!("type\t{}", EntryKind::of(&meta).label());
    println!("size\t{}", meta.size);
    println!("mtime_ms\t{}", meta.mtime_ms);
    if let Some(id) = meta.id {
        println!("id\t{id}");
    }
    if let Some(md5) = meta.content_md5 {
        println!("md5\t{md5}");
    }
    Ok(())
}

pub(crate) fn cat(target: &Target) -> Result<(), String> {
    let mut r = target
        .backend
        .open_read(&target.path)
        .map_err(|error| explain_read_error(target, error))?;
    let mut out = io::stdout().lock();
    io::copy(&mut r, &mut out).map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())
}

/// A read through a link that fails names the link as the cause.
pub(super) fn explain_read_error(target: &Target, error: io::Error) -> String {
    match target.backend.stat(&target.path) {
        Ok(meta) if meta.is_symlink => format!(
            "{} is a link whose target does not exist, is not a file, or cannot be read: {error}",
            target.path
        ),
        _ => error.to_string(),
    }
}

/// `se cp`: prints one stderr line per skipped link or special file.
pub(crate) fn copy(src: &Target, dst: &Target, recursive: bool, force: bool) -> Result<(), String> {
    let outcome = copy_observed(src, dst, recursive, force, &TreeObserver::default())?;
    print_notices(&outcome);
    Ok(())
}

/// `se cp` for background callers: the same checks and pipeline with
/// cancellation and progress.
pub(super) fn copy_observed(
    src: &Target,
    dst: &Target,
    recursive: bool,
    force: bool,
    observer: &TreeObserver,
) -> Result<TreeOutcome, String> {
    let resolved_dst = resolve_destination(dst)?;
    let dst = resolved_dst.as_ref().unwrap_or(dst);
    let meta = src.backend.stat(&src.path).map_err(|e| e.to_string())?;
    let (resolved_src, name) = match source_root(src, &meta)? {
        SourceRoot::Plain => (None, None),
        SourceRoot::Resolved { target, name } => (Some(target), Some(name)),
        SourceRoot::RemoteLink => {
            let (_, dst_path) = transfer_plan(src, dst)?;
            return copy_remote_link(src, dst, &dst_path, force, observer);
        }
    };
    let src = resolved_src.as_ref().unwrap_or(src);
    let (source_snapshot, dst_path) = transfer_plan_as(src, dst, name.as_deref())?;
    let receipt = copy_entry_with(
        &*src.backend,
        &src.path,
        &*dst.backend,
        &dst_path,
        &TreeRequest {
            recursive,
            force,
            expected_root: Some(&source_snapshot),
            destination_check: None,
            observer,
        },
    )?;
    observer.set_phase(TreePhase::Done);
    Ok(TreeOutcome {
        destination: receipt.destination().to_string(),
        omitted: receipt.omissions(),
        kept: Vec::new(),
    })
}

/// `se mv`: prints skipped entries and the source directories kept for them.
pub(crate) fn move_path(
    src: &Target,
    dst: &Target,
    recursive: bool,
    force: bool,
) -> Result<(), String> {
    let outcome = move_observed(src, dst, recursive, force, &TreeObserver::default())?;
    print_notices(&outcome);
    Ok(())
}

/// `se mv` for background callers. A cancellation that arrives after the
/// copy completed keeps the source: the removal phase is never interrupted
/// halfway. A source link is renamed, never followed.
pub(super) fn move_observed(
    src: &Target,
    dst: &Target,
    recursive: bool,
    force: bool,
    observer: &TreeObserver,
) -> Result<TreeOutcome, String> {
    let resolved_dst = resolve_destination(dst)?;
    let dst = resolved_dst.as_ref().unwrap_or(dst);
    let (meta, dst_path) = transfer_plan(src, dst)?;
    if meta.is_dir && !recursive {
        return Err(format!("{} is a directory; pass --recursive", src.path));
    }
    if try_rename_fast(src, dst, &meta, &dst_path, force)? {
        observer.set_phase(TreePhase::Done);
        return Ok(TreeOutcome::complete(dst_path));
    }
    if meta.is_symlink {
        return Err(link_move_refused(&src.path));
    }
    let receipt = copy_entry_with(
        &*src.backend,
        &src.path,
        &*dst.backend,
        &dst_path,
        &TreeRequest {
            recursive,
            force,
            expected_root: Some(&meta),
            destination_check: None,
            observer,
        },
    )?;
    observer.check()?;
    observer.set_phase(TreePhase::RemovingSource);
    let kept = remove_copied_source(&*src.backend, &receipt)?;
    observer.set_phase(TreePhase::Done);
    Ok(TreeOutcome {
        destination: receipt.destination().to_string(),
        omitted: receipt.omissions(),
        kept,
    })
}

fn print_notices(outcome: &TreeOutcome) {
    let mut err = io::stderr().lock();
    for line in outcome.notices() {
        let _ = writeln!(err, "se: {line}");
    }
}

pub(crate) fn remove(
    target: &Target,
    recursive: bool,
    force: bool,
    no_preserve_root: bool,
) -> Result<(), String> {
    if !force {
        return Err("rm requires --force".into());
    }
    if target.is_preserved_root() && !no_preserve_root {
        return Err(
            "refusing to remove a filesystem or configured connection root; pass --no-preserve-root to override"
                .into(),
        );
    }
    remove_existing(&*target.backend, &target.path, recursive)
}

pub(super) fn parent_of(path: &str) -> Option<String> {
    let path = path.trim_end_matches('/');
    if path
        .strip_prefix("//")
        .is_some_and(|rest| rest.split('/').filter(|part| !part.is_empty()).count() <= 2)
    {
        // A UNC share (`//server/share`) is a filesystem root. Walking to
        // `//server` would probe a server namespace rather than a directory.
        return None;
    }
    path.rsplit_once('/').map(|(p, _)| {
        if p.is_empty() {
            "/".to_string()
        } else if p.len() == 2 && p.as_bytes()[1] == b':' {
            format!("{p}/")
        } else {
            p.to_string()
        }
    })
}

pub(super) fn join(parent: &str, name: &str) -> String {
    let name = name.trim_matches('/');
    if parent == "/" {
        format!("/{name}")
    } else if parent.ends_with('/') {
        format!("{parent}{name}")
    } else {
        format!("{parent}/{name}")
    }
}

#[allow(dead_code)]
fn local_path(path: &str) -> PathBuf {
    PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use crate::cli::target::Target;
    use crate::vfs::{BackendHandle, LocalBackend};

    use super::{parent_of, remove};
    use crate::cli::tree_ops::{copy_entry, remove_existing};

    #[test]
    fn parent_handles_root_children() {
        assert_eq!(parent_of("/a.txt").as_deref(), Some("/"));
        assert_eq!(parent_of("/dir/a.txt").as_deref(), Some("/dir"));
        assert_eq!(parent_of("C:/a.txt").as_deref(), Some("C:/"));
        assert_eq!(
            parent_of("//server/share/f.txt").as_deref(),
            Some("//server/share")
        );
        assert_eq!(parent_of("//server/share"), None);
    }

    #[test]
    fn copy_file_requires_force_for_overwrite() {
        let temp = TempRoot::new("copy-overwrite");
        let src = temp.file("src.txt", b"new");
        let dst = temp.file("dst.txt", b"old");
        let backend = LocalBackend::new("/");

        let err = copy_entry(&backend, &src, &backend, &dst, false, false)
            .err()
            .expect("overwrite without --force must fail");
        assert!(err.contains("--force"));

        copy_entry(&backend, &src, &backend, &dst, false, true).unwrap();
        assert_eq!(fs::read(vfs_to_os(&dst)).unwrap(), b"new");
    }

    #[test]
    fn recursive_copy_is_required_for_directories() {
        let temp = TempRoot::new("copy-dir");
        let src_dir = temp.dir("src");
        fs::write(vfs_to_os(&format!("{src_dir}/child.txt")), b"child").unwrap();
        let dst_dir = temp.path("dst");
        let backend = LocalBackend::new("/");

        let err = copy_entry(&backend, &src_dir, &backend, &dst_dir, false, false)
            .err()
            .expect("directory copy without --recursive must fail");
        assert!(err.contains("--recursive"));

        copy_entry(&backend, &src_dir, &backend, &dst_dir, true, false).unwrap();
        assert_eq!(
            fs::read(vfs_to_os(&format!("{dst_dir}/child.txt"))).unwrap(),
            b"child"
        );
    }

    #[test]
    fn rm_requires_force_and_recursive_for_directories() {
        let temp = TempRoot::new("rm-guards");
        let dir = temp.dir("dir");
        fs::write(vfs_to_os(&format!("{dir}/child.txt")), b"child").unwrap();
        let backend: BackendHandle = Arc::new(LocalBackend::new("/"));
        let target = Target::with_backend_key(backend.clone(), dir.clone(), "local");

        assert_eq!(
            remove(&target, true, false, false).unwrap_err(),
            "rm requires --force"
        );
        let err = remove_existing(&*backend, &dir, false).unwrap_err();
        assert!(err.contains("--recursive"));

        remove(&target, true, true, false).unwrap();
        assert!(!vfs_to_os(&dir).exists());
    }

    #[test]
    fn rm_preserves_roots_without_explicit_override() {
        let temp = TempRoot::new("rm-preserve-root");
        let dir = temp.dir("configured-root");
        let backend: BackendHandle = Arc::new(LocalBackend::new("/"));
        let target = Target::with_backend_key_preserved_root(backend, dir.clone(), "local");

        let error = remove(&target, true, true, false).unwrap_err();
        assert!(error.contains("--no-preserve-root"));
        assert!(vfs_to_os(&dir).exists());

        remove(&target, true, true, true).unwrap();
        assert!(!vfs_to_os(&dir).exists());
    }

    struct TempRoot {
        path: std::path::PathBuf,
    }

    impl TempRoot {
        fn new(name: &str) -> Self {
            let unique = format!(
                "smart-explorer-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self, rel: &str) -> String {
            self.path.join(rel).to_string_lossy().replace('\\', "/")
        }

        fn file(&self, rel: &str, content: &[u8]) -> String {
            let path = self.path(rel);
            fs::write(vfs_to_os(&path), content).unwrap();
            path
        }

        fn dir(&self, rel: &str) -> String {
            let path = self.path(rel);
            fs::create_dir_all(vfs_to_os(&path)).unwrap();
            path
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn vfs_to_os(path: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR))
    }
}
