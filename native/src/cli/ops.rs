use super::target::Target;
use crate::agent_proto::SearchSpec;
use crate::vfs::{Backend, SearchHit, VfsMeta};
use crossbeam_channel::unbounded;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

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
    let meta = src.backend.stat(&src.path).map_err(|e| e.to_string())?;
    let dst_path = destination_path(dst, &meta);
    copy_entry(
        &*src.backend,
        &src.path,
        &*dst.backend,
        &dst_path,
        recursive,
        force,
    )
}

pub(crate) fn move_path(
    src: &Target,
    dst: &Target,
    recursive: bool,
    force: bool,
) -> Result<(), String> {
    copy(src, dst, recursive, force)?;
    remove_existing(&*src.backend, &src.path, recursive)
}

pub(crate) fn remove(target: &Target, recursive: bool, force: bool) -> Result<(), String> {
    if !force {
        return Err("rm requires --force".into());
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
        let (tx, rx) = unbounded();
        let cancel = AtomicBool::new(false);
        if target.backend.search(&target.path, &spec, tx, &cancel) {
            for hit in rx {
                print_hit(hit);
            }
            return Ok(());
        }
    }
    let mut count = 0u64;
    fallback_search(&*target.backend, &target.path, "", &spec, &mut count)
}

fn destination_path(dst: &Target, src_meta: &VfsMeta) -> String {
    match dst.backend.stat(&dst.path) {
        Ok(meta) if meta.is_dir => join(&dst.path, &src_meta.name),
        _ => dst.path.clone(),
    }
}

fn copy_entry(
    src: &dyn Backend,
    src_path: &str,
    dst: &dyn Backend,
    dst_path: &str,
    recursive: bool,
    force: bool,
) -> Result<(), String> {
    let meta = src.stat(src_path).map_err(|e| e.to_string())?;
    if meta.is_dir {
        if !recursive {
            return Err(format!("{src_path} is a directory; pass --recursive"));
        }
        match dst.stat(dst_path) {
            Ok(existing) if !existing.is_dir => {
                return Err(format!(
                    "destination exists and is not a directory: {dst_path}"
                ))
            }
            Ok(_) => {}
            Err(_) => dst.mkdir_all(dst_path).map_err(|e| e.to_string())?,
        }
        for child in src.list_dir(src_path).map_err(|e| e.to_string())? {
            if child.is_symlink {
                continue;
            }
            let sp = join(src_path, &child.name);
            let dp = join(dst_path, &child.name);
            copy_entry(src, &sp, dst, &dp, recursive, force)?;
        }
        return Ok(());
    }

    if dst.stat(dst_path).is_ok() && !force {
        return Err(format!("destination exists; pass --force: {dst_path}"));
    }
    if let Some(parent) = parent_of(dst_path) {
        let _ = dst.mkdir_all(&parent);
    }
    let mut r = src.open_read(src_path).map_err(|e| e.to_string())?;
    let mut w = dst.open_write(dst_path).map_err(|e| e.to_string())?;
    io::copy(&mut r, &mut w).map_err(|e| e.to_string())?;
    w.flush().map_err(|e| e.to_string())
}

fn remove_existing(be: &dyn Backend, path: &str, recursive: bool) -> Result<(), String> {
    let meta = be.stat(path).map_err(|e| e.to_string())?;
    if meta.is_dir {
        if !recursive {
            return Err(format!("{path} is a directory; pass --recursive"));
        }
        remove_tree(be, path)
    } else {
        be.remove_file(path).map_err(|e| e.to_string())
    }
}

fn remove_tree(be: &dyn Backend, path: &str) -> Result<(), String> {
    for child in be.list_dir(path).map_err(|e| e.to_string())? {
        let p = join(path, &child.name);
        if child.is_symlink {
            be.remove_file(&p).map_err(|e| e.to_string())?;
            continue;
        }
        let meta = be.stat(&p).map_err(|e| e.to_string())?;
        if meta.is_dir {
            remove_tree(be, &p)?;
        } else {
            be.remove_file(&p).map_err(|e| e.to_string())?;
        }
    }
    be.remove_dir(path).map_err(|e| e.to_string())
}

fn fallback_search(
    be: &dyn Backend,
    dir: &str,
    rel_dir: &str,
    spec: &SearchSpec,
    count: &mut u64,
) -> Result<(), String> {
    for child in be.list_dir(dir).map_err(|e| e.to_string())? {
        if spec.max_results != 0 && *count >= spec.max_results {
            break;
        }
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
        if child.is_dir && !child.is_symlink {
            fallback_search(be, &join(dir, &child.name), &rel, spec, count)?;
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

fn parent_of(path: &str) -> Option<String> {
    let path = path.trim_end_matches('/');
    path.rsplit_once('/').map(|(p, _)| {
        if p.is_empty() {
            "/".to_string()
        } else {
            p.to_string()
        }
    })
}

fn join(parent: &str, name: &str) -> String {
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

    use super::{copy_entry, glob_match, parent_of, remove, remove_existing};

    #[test]
    fn glob_matches_case_insensitive() {
        assert!(glob_match("*.TXT", "note.txt"));
        assert!(!glob_match("a?.txt", "abc.txt"));
    }

    #[test]
    fn parent_handles_root_children() {
        assert_eq!(parent_of("/a.txt").as_deref(), Some("/"));
        assert_eq!(parent_of("/dir/a.txt").as_deref(), Some("/dir"));
    }

    #[test]
    fn copy_file_requires_force_for_overwrite() {
        let temp = TempRoot::new("copy-overwrite");
        let src = temp.file("src.txt", b"new");
        let dst = temp.file("dst.txt", b"old");
        let backend = LocalBackend::new("/");

        let err = copy_entry(&backend, &src, &backend, &dst, false, false).unwrap_err();
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

        let err = copy_entry(&backend, &src_dir, &backend, &dst_dir, false, false).unwrap_err();
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
        let target = Target {
            backend: backend.clone(),
            path: dir.clone(),
            net: None,
        };

        assert_eq!(
            remove(&target, true, false).unwrap_err(),
            "rm requires --force"
        );
        let err = remove_existing(&*backend, &dir, false).unwrap_err();
        assert!(err.contains("--recursive"));

        remove(&target, true, true).unwrap();
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
