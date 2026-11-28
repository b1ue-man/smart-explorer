use super::entry_kind::EntryKind;
use super::target::Target;
use super::tree_ops::{remove_existing, TreeOutcome};
use std::io::{self, Write};
use std::path::PathBuf;

/// `se ls`: entries the listing cannot name (not valid Unicode, a name
/// Win32 cannot address, U9) are named on stderr instead of failing it.
pub(crate) fn list(target: &Target) -> Result<(), String> {
    let listing =
        super::tree_guard::listing(&*target.backend, &target.path).map_err(|e| e.to_string())?;
    for entry in listing.entries {
        // Second column: `l` for links (kept for scripts that read it).
        let kind = match EntryKind::of(&entry) {
            EntryKind::Dir => 'd',
            EntryKind::Special => 's',
            EntryKind::File | EntryKind::Link => '-',
        };
        let symlink = if entry.is_symlink { "l" } else { "-" };
        println!("{kind}{symlink}\t{}\t{}", entry.size, entry.name);
    }
    report_omitted(&target.path, &listing.omitted);
    Ok(())
}

/// One stderr line per entry of `directory` a tolerant listing left out.
pub(super) fn report_omitted(directory: &str, omitted: &[crate::vfs::VfsOmission]) {
    let mut err = io::stderr().lock();
    for omission in omitted {
        let _ = writeln!(
            err,
            "se: nicht aufgelistet: {}: {}",
            join(directory, &omission.rel),
            omission.detail
        );
    }
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
    cat_observed(target, &super::tree_observer::TreeObserver::default())
}

pub(crate) fn cat_observed(
    target: &Target,
    observer: &super::tree_observer::TreeObserver,
) -> Result<(), String> {
    crate::source_media::with_cancellation(observer.cancel_flag(), || {
        observer.check()?;
        let mut reader = target
            .backend
            .open_read(&target.path)
            .map_err(|error| observer.explain(explain_read_error(target, error)))?;
        let mut output = io::stdout().lock();
        io::copy(&mut reader, &mut output).map_err(|error| observer.explain(error.to_string()))?;
        output
            .flush()
            .map_err(|error| observer.explain(error.to_string()))
    })
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

#[path = "ops_copy.rs"]
mod copying;
#[path = "ops_move.rs"]
mod moving;
#[cfg(test)]
pub(crate) use copying::copy;
pub(super) use copying::{copy_observed, copy_observed_in_job, copy_observed_with_folders};
pub(crate) use copying::{copy_with_folder_stats, copy_with_stats};
pub(super) use moving::{move_observed, move_observed_in_job};

/// `se mv`. A move to the same name in another case in the same folder
/// (`STICK/foto.jpg` → `STICK/Foto.jpg`) is a rename in two steps (U10).
pub(crate) fn move_path(
    src: &Target,
    dst: &Target,
    recursive: bool,
    force: bool,
) -> Result<(), String> {
    if super::transfer::try_case_rename(src, dst, recursive)? {
        return Ok(());
    }
    moving::move_path(src, dst, recursive, force)
}

/// `se mv` with a folder policy or `--resume-job`; a pure change of case
/// (new runs only) as in [`move_path`].
pub(crate) fn move_with_folders(
    src: &Target,
    dst: &Target,
    recursive: bool,
    force: bool,
    folders: Option<crate::transfer::FolderConflict>,
    resume: Option<&str>,
) -> Result<(), String> {
    if resume.is_none() && super::transfer::try_case_rename(src, dst, recursive)? {
        if folders.is_some() {
            println!("{}", dst.path);
        }
        return Ok(());
    }
    moving::move_with_folders(src, dst, recursive, force, folders, resume)
}

/// The MCP `move`; a pure change of case as in [`move_path`].
pub(super) fn move_observed_with_folders(
    src: &Target,
    dst: &Target,
    recursive: bool,
    force: bool,
    observer: &super::tree_observer::TreeObserver,
    folders: crate::transfer::FolderConflict,
) -> Result<TreeOutcome, String> {
    if let Some(outcome) = case_renamed(src, dst, recursive, observer)? {
        return Ok(outcome);
    }
    moving::move_observed_with_folders(src, dst, recursive, force, observer, folders)
}

fn case_renamed(
    src: &Target,
    dst: &Target,
    recursive: bool,
    observer: &super::tree_observer::TreeObserver,
) -> Result<Option<TreeOutcome>, String> {
    observer.check()?;
    if !super::transfer::try_case_rename(src, dst, recursive)? {
        return Ok(None);
    }
    observer.set_phase(super::tree_observer::TreePhase::Done);
    Ok(Some(TreeOutcome::complete(dst.path.clone())))
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

/// The root of a Windows verbatim or device path as a prefix of `path`:
/// `//?/E:`, `//./E:`, `//?/Volume{…}`, `//./<device>`, `//?/UNC/server/share`
/// and `//?/GLOBALROOT/Device/<name>`, with whether the folder at that root
/// is written with a trailing `/` (a drive or volume: without it the path
/// names the volume device itself; a UNC share stays like `//server/share`).
pub(super) fn verbatim_root(path: &str) -> Option<(&str, bool)> {
    let rest = path
        .strip_prefix("//?/")
        .or_else(|| path.strip_prefix("//./"))?;
    let first = rest.split('/').next().filter(|part| !part.is_empty())?;
    let (components, slash) = if first.eq_ignore_ascii_case("UNC") {
        (3, false)
    } else if first.eq_ignore_ascii_case("GLOBALROOT") {
        (3, true)
    } else {
        (1, true)
    };
    let length = rest
        .split('/')
        .take(components)
        .map(|part| part.len() + 1)
        .sum::<usize>()
        - 1;
    Some((&path[..4 + length], slash))
}

/// Whether `path` (trailing `/` ignored) is exactly a verbatim or device root.
pub(super) fn is_verbatim_root(path: &str) -> bool {
    let path = path.trim_end_matches('/');
    verbatim_root(path).is_some_and(|(root, _)| root.len() == path.len())
}

pub(super) fn parent_of(path: &str) -> Option<String> {
    let path = path.trim_end_matches('/');
    if let Some((root, slash)) = verbatim_root(path) {
        if path.len() <= root.len() {
            return None;
        }
        let parent = path.rsplit_once('/').map_or(root, |(parent, _)| parent);
        return Some(if parent.len() > root.len() {
            parent.to_string()
        } else if slash {
            format!("{root}/")
        } else {
            root.to_string()
        });
    }
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
#[path = "ops_removable_tests.rs"]
mod removable_tests;

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use crate::cli::target::Target;
    use crate::vfs::{BackendHandle, LocalBackend};

    use super::{is_verbatim_root, parent_of, remove};
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
    fn wechselmedium_task_verbatim_roots_keep_their_trailing_slash() {
        assert_eq!(parent_of("//?/E:/a.txt").as_deref(), Some("//?/E:/"));
        assert_eq!(parent_of("//./E:/dir/a").as_deref(), Some("//./E:/dir"));
        assert_eq!(
            parent_of("//?/Volume{0b3e8f27-1c2d-4e5f-8a9b-0c1d2e3f4a5b}/foto.jpg").as_deref(),
            Some("//?/Volume{0b3e8f27-1c2d-4e5f-8a9b-0c1d2e3f4a5b}/")
        );
        assert_eq!(
            parent_of("//?/UNC/server/share/f.txt").as_deref(),
            Some("//?/UNC/server/share")
        );
        assert_eq!(
            parent_of("//?/GLOBALROOT/Device/HarddiskVolume3/f").as_deref(),
            Some("//?/GLOBALROOT/Device/HarddiskVolume3/")
        );
        for root in [
            "//?/E:",
            "//?/E:/",
            "//./E:",
            "//?/Volume{0b3e8f27-1c2d-4e5f-8a9b-0c1d2e3f4a5b}/",
            "//?/UNC/server/share",
            "//?/GLOBALROOT/Device/HarddiskVolume3",
        ] {
            assert_eq!(parent_of(root), None, "{root} is a root");
            assert!(is_verbatim_root(root), "{root}");
        }
        assert!(!is_verbatim_root("//?/E:/dir"));
        assert!(!is_verbatim_root("//server/share"));
        // The parent of a child of a root is that root, never its device.
        let mut current = "//?/E:/a/b".to_string();
        let mut chain = Vec::new();
        while let Some(parent) = parent_of(&current) {
            chain.push(parent.clone());
            current = parent;
        }
        assert_eq!(chain, ["//?/E:/a", "//?/E:/"]);
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
