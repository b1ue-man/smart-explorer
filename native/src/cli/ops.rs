use super::target::Target;
use super::transfer::{transfer_plan, try_rename_fast};
use super::tree_ops::{copy_entry_from_snapshot, remove_copied_source, remove_existing};
use crate::agent_proto::SearchSpec;
use crate::vfs::{Backend, SearchHit};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_SEARCH_NODES: u64 = 1_000_000;
const MAX_SEARCH_TEXT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SEARCH_DEPTH: usize = 512;

#[derive(Default)]
struct SearchBudget {
    nodes: u64,
    text_bytes: u64,
}

impl SearchBudget {
    fn record(&mut self, path: &str, depth: usize) -> Result<(), String> {
        if depth > MAX_SEARCH_DEPTH {
            return Err(format!("search tree exceeds {MAX_SEARCH_DEPTH} levels"));
        }
        self.nodes = self.nodes.saturating_add(1);
        self.text_bytes = self.text_bytes.saturating_add(path.len() as u64);
        if self.nodes > MAX_SEARCH_NODES || self.text_bytes > MAX_SEARCH_TEXT_BYTES {
            return Err("search tree exceeds its bounded collection budget".to_string());
        }
        Ok(())
    }
}

pub(crate) fn list(target: &Target) -> Result<(), String> {
    for entry in target
        .backend
        .list_dir(&target.path)
        .map_err(|e| e.to_string())?
    {
        let kind = if entry.is_dir { "d" } else { "-" };
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
    println!("type\t{}", if meta.is_dir { "dir" } else { "file" });
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
        .map_err(|e| e.to_string())?;
    let mut out = io::stdout().lock();
    io::copy(&mut r, &mut out).map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())
}

pub(crate) fn copy(src: &Target, dst: &Target, recursive: bool, force: bool) -> Result<(), String> {
    let (source_snapshot, dst_path) = transfer_plan(src, dst)?;
    copy_entry_from_snapshot(
        &*src.backend,
        &src.path,
        &*dst.backend,
        &dst_path,
        recursive,
        force,
        Some(&source_snapshot),
    )
    .map(|_| ())
}

pub(crate) fn move_path(
    src: &Target,
    dst: &Target,
    recursive: bool,
    force: bool,
) -> Result<(), String> {
    let (meta, dst_path) = transfer_plan(src, dst)?;
    if meta.is_dir && !recursive {
        return Err(format!("{} is a directory; pass --recursive", src.path));
    }
    if try_rename_fast(src, dst, &meta, &dst_path, force)? {
        return Ok(());
    }
    let receipt = copy_entry_from_snapshot(
        &*src.backend,
        &src.path,
        &*dst.backend,
        &dst_path,
        recursive,
        force,
        Some(&meta),
    )?;
    remove_copied_source(&*src.backend, &receipt)
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

pub(crate) fn search(
    target: &Target,
    query: &str,
    glob: bool,
    max_results: u64,
    want_dirs: bool,
) -> Result<(), String> {
    let spec = SearchSpec {
        query: query.to_string(),
        glob,
        min_size: 0,
        max_size: 0,
        max_results,
        want_dirs,
    };
    if target.backend.supports_search() {
        let (tx, rx) = crossbeam_channel::bounded(1024);
        let cancel = AtomicBool::new(false);
        let mut invalid = None;
        let mut received = false;
        let ran = std::thread::scope(|scope| {
            let handle = scope.spawn(|| target.backend.search(&target.path, &spec, tx, &cancel));
            for hit in rx {
                received = true;
                if invalid.is_some() {
                    continue;
                }
                if let Err(error) = crate::agent_proto::ValidatedRelativePath::parse(&hit.rel) {
                    cancel.store(true, Ordering::Relaxed);
                    invalid = Some(error.to_string());
                } else {
                    print_hit(hit);
                }
            }
            handle
                .join()
                .map_err(|_| "server-side search worker panicked".to_string())
        })?;
        if let Some(error) = invalid {
            return Err(error);
        }
        match ran.map_err(|error| error.to_string())? {
            true => return Ok(()),
            false if received => {
                return Err(
                    "backend reported search unsupported after streaming results".to_string(),
                );
            }
            false => {}
        }
    }
    let mut count = 0u64;
    fallback_search(
        &*target.backend,
        &target.path,
        "",
        &spec,
        &mut count,
        0,
        &mut SearchBudget::default(),
    )
}

fn fallback_search(
    be: &dyn Backend,
    dir: &str,
    rel_dir: &str,
    spec: &SearchSpec,
    count: &mut u64,
    depth: usize,
    budget: &mut SearchBudget,
) -> Result<(), String> {
    budget.record(dir, depth)?;
    let mut names = std::collections::HashSet::new();
    for child in be.list_dir(dir).map_err(|e| e.to_string())? {
        if spec.max_results != 0 && *count >= spec.max_results {
            break;
        }
        crate::vfs::validate_child_name(&child.name).map_err(|error| error.to_string())?;
        if !names.insert(child.name.clone()) {
            return Err(format!(
                "backend returned duplicate child name in {dir}: {:?}",
                child.name
            ));
        }
        if child.is_symlink {
            return Err(format!(
                "link-like search entry is unsupported: {dir}/{}",
                child.name
            ));
        }
        budget.record(&join(dir, &child.name), depth + 1)?;
        let rel = if rel_dir.is_empty() {
            child.name.clone()
        } else {
            format!("{rel_dir}/{}", child.name)
        };
        if matches_spec(&child.name, child.is_dir, child.size, spec) {
            *count += 1;
            print_hit(SearchHit {
                rel: rel.clone(),
                is_dir: child.is_dir,
                size: child.size,
                mtime_ms: child.mtime_ms,
            });
        }
        if child.is_dir {
            fallback_search(
                be,
                &join(dir, &child.name),
                &rel,
                spec,
                count,
                depth + 1,
                budget,
            )?;
        }
    }
    Ok(())
}

fn matches_spec(name: &str, is_dir: bool, size: u64, spec: &SearchSpec) -> bool {
    if is_dir && !spec.want_dirs {
        return false;
    }
    if !is_dir && ((spec.max_size != 0 && size > spec.max_size) || size < spec.min_size) {
        return false;
    }
    if spec.query.is_empty() {
        return true;
    }
    if spec.glob {
        glob_match(&spec.query, name)
    } else {
        name.to_lowercase().contains(&spec.query.to_lowercase())
    }
}

fn glob_match(pat: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (
        pat.to_lowercase().chars().collect(),
        text.to_lowercase().chars().collect(),
    );
    let (mut pi, mut ti, mut star, mut mark) = (0usize, 0usize, usize::MAX, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = pi;
            mark = ti;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn print_hit(hit: SearchHit) {
    println!(
        "{}\t{}\t{}",
        if hit.is_dir { "d" } else { "-" },
        hit.size,
        hit.rel
    );
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

    use super::{glob_match, parent_of, remove};
    use crate::cli::tree_ops::{copy_entry, remove_existing};

    #[test]
    fn glob_matches_case_insensitive() {
        assert!(glob_match("*.TXT", "note.txt"));
        assert!(!glob_match("a?.txt", "abc.txt"));
    }

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
