use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use crate::agent_proto::{Frame, SearchSpec, WireNode, CHUNK};
use crate::vfs::BackendHandle;

use super::backend_budget::WalkBudget;
use super::backend_server::{emit, Sink};

fn join_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{}/{}", parent.trim_end_matches('/'), name)
    }
}

fn rel_join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

fn canceled(operation: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, format!("{operation} canceled"))
}

pub(super) fn handle_walk_tree_backend(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    root: &str,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let files = Arc::new(AtomicU64::new(0));
    let bytes = Arc::new(AtomicU64::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let sink2 = sink.clone();
    let files2 = files.clone();
    let bytes2 = bytes.clone();
    let done2 = done.clone();
    let emitter = std::thread::Builder::new()
        .name("daemon-backend-walk-progress".into())
        .spawn(move || -> io::Result<()> {
            while !done2.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(250));
                if done2.load(Ordering::Relaxed) {
                    break;
                }
                emit(
                    &sink2,
                    id,
                    &Frame::Progress {
                        done: files2.load(Ordering::Relaxed),
                        total: bytes2.load(Ordering::Relaxed),
                    },
                )?;
            }
            Ok(())
        })
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("daemon walk progress worker could not start: {error}"),
            )
        })?;
    let mut budget = WalkBudget::default();
    let result = TreeWalker {
        backend,
        budget: &mut budget,
        cancel,
        files: &files,
        bytes: &bytes,
    }
    .walk_node(root, node_name(root), 0);
    done.store(true, Ordering::Relaxed);
    let emitter_result = match emitter.join() {
        Ok(result) => result,
        Err(payload) => Err(io::Error::other(format!(
            "daemon walk progress worker panicked: {}",
            panic_message(&payload)
        ))),
    };
    let tree = combine_walk_and_emitter(result, emitter_result)?;
    emit(sink, id, &Frame::Tree(tree))
}

fn combine_walk_and_emitter<T>(walk: io::Result<T>, emitter: io::Result<()>) -> io::Result<T> {
    match (walk, emitter) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(walk_error), Err(emitter_error)) => Err(io::Error::new(
            walk_error.kind(),
            format!("backend walk failed ({walk_error}); progress worker failed: {emitter_error}"),
        )),
    }
}

fn panic_message<'a>(payload: &'a (dyn std::any::Any + Send + 'static)) -> &'a str {
    payload
        .downcast_ref::<&'static str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic payload")
}

fn node_name(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

struct TreeWalker<'a> {
    backend: &'a BackendHandle,
    budget: &'a mut WalkBudget,
    cancel: &'a AtomicBool,
    files: &'a AtomicU64,
    bytes: &'a AtomicU64,
}

impl TreeWalker<'_> {
    fn walk_node(&mut self, path: &str, name: String, depth: usize) -> io::Result<WireNode> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(canceled("daemon backend tree walk"));
        }
        self.budget.record(path, depth)?;
        let meta = self.backend.stat(path)?;
        if meta.is_symlink {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("backend tree walk does not follow link-like entry: {path}"),
            ));
        }
        if !meta.is_dir {
            self.files.fetch_add(1, Ordering::Relaxed);
            self.bytes.fetch_add(meta.size, Ordering::Relaxed);
            return Ok(WireNode {
                name,
                size: meta.size,
                is_dir: false,
                children: Vec::new(),
            });
        }
        let mut total = 0u64;
        let mut children = Vec::new();
        let mut names = std::collections::HashSet::new();
        for child in self.backend.list_dir(path)? {
            crate::vfs::validate_child_name(&child.name)?;
            if !names.insert(child.name.clone()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "backend returned duplicate child name in {path}: {:?}",
                        child.name
                    ),
                ));
            }
            if child.is_symlink {
                continue;
            }
            let child_path = join_path(path, &child.name);
            let node = self.walk_node(&child_path, child.name.clone(), depth + 1)?;
            total = total
                .checked_add(node.size)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "tree size overflow"))?;
            children.push(node);
        }
        Ok(WireNode {
            name,
            size: total,
            is_dir: true,
            children,
        })
    }
}

pub(super) fn remove_one_backend(backend: &BackendHandle, path: &str) -> io::Result<()> {
    let meta = backend.stat(path)?;
    if meta.is_dir && !meta.is_symlink {
        backend.remove_dir(path)
    } else {
        backend.remove_file(path)
    }
}

fn glob_match(pattern: &str, value: &str) -> bool {
    let (pattern, value): (Vec<char>, Vec<char>) = (
        pattern.to_lowercase().chars().collect(),
        value.to_lowercase().chars().collect(),
    );
    let (mut pi, mut vi, mut star, mut mark) = (0usize, 0usize, usize::MAX, 0usize);
    while vi < value.len() {
        if pi < pattern.len() && (pattern[pi] == '?' || pattern[pi] == value[vi]) {
            pi += 1;
            vi += 1;
        } else if pi < pattern.len() && pattern[pi] == '*' {
            star = pi;
            mark = vi;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            vi = mark;
        } else {
            return false;
        }
    }
    while pi < pattern.len() && pattern[pi] == '*' {
        pi += 1;
    }
    pi == pattern.len()
}

fn matches_spec(name: &str, is_dir: bool, size: u64, spec: &SearchSpec) -> bool {
    if is_dir && !spec.want_dirs {
        return false;
    }
    if !is_dir && (size < spec.min_size || (spec.max_size != 0 && size > spec.max_size)) {
        return false;
    }
    if spec.query.is_empty() {
        true
    } else if spec.glob {
        glob_match(&spec.query, name)
    } else {
        name.to_lowercase().contains(&spec.query.to_lowercase())
    }
}

pub(super) fn handle_search_backend(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    root: &str,
    spec: &SearchSpec,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let mut count = 0u64;
    let mut budget = WalkBudget::default();
    budget.record(root, 0)?;
    let mut stack = vec![(root.to_string(), String::new(), 0usize)];
    while let Some((directory, relative_directory, depth)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return Err(canceled("daemon backend search"));
        }
        require_plain_directory(backend, &directory)?;
        let mut names = std::collections::HashSet::new();
        for child in backend.list_dir(&directory)? {
            crate::vfs::validate_child_name(&child.name)?;
            if !names.insert(child.name.clone()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "backend returned duplicate child name in {directory}: {:?}",
                        child.name
                    ),
                ));
            }
            let child_path = join_path(&directory, &child.name);
            budget.record(&child_path, depth + 1)?;
            if child.is_symlink {
                continue;
            }
            let relative = rel_join(&relative_directory, &child.name);
            if child.is_dir {
                stack.push((child_path, relative.clone(), depth + 1));
            }
            if matches_spec(&child.name, child.is_dir, child.size, spec) {
                emit(
                    sink,
                    id,
                    &Frame::Match {
                        rel: relative,
                        is_dir: child.is_dir,
                        size: child.size,
                        mtime_ms: child.mtime_ms,
                    },
                )?;
                count += 1;
                if spec.max_results != 0 && count >= spec.max_results {
                    return emit(sink, id, &Frame::End);
                }
            }
        }
    }
    emit(sink, id, &Frame::End)
}

pub(super) fn handle_walk_hashed_backend(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    root: &str,
    want_hash: bool,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let mut budget = WalkBudget::default();
    budget.record(root, 0)?;
    let mut stack = vec![(root.to_string(), String::new(), 0usize)];
    while let Some((directory, relative_directory, depth)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return Err(canceled("daemon backend hash walk"));
        }
        require_plain_directory(backend, &directory)?;
        let mut names = std::collections::HashSet::new();
        for child in backend.list_dir(&directory)? {
            crate::vfs::validate_child_name(&child.name)?;
            if !names.insert(child.name.clone()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "backend returned duplicate child name in {directory}: {:?}",
                        child.name
                    ),
                ));
            }
            let path = join_path(&directory, &child.name);
            budget.record(&path, depth + 1)?;
            if child.is_symlink {
                continue;
            }
            let relative = rel_join(&relative_directory, &child.name);
            if child.is_dir {
                emit(
                    sink,
                    id,
                    &Frame::HashEntry {
                        rel: relative.clone(),
                        is_dir: true,
                        size: 0,
                        mtime_ms: child.mtime_ms,
                        md5: None,
                    },
                )?;
                stack.push((path, relative, depth + 1));
            } else {
                let md5 = if want_hash {
                    Some(match child.content_md5.clone() {
                        Some(md5) => md5,
                        None => md5_backend(backend, &path, cancel)?,
                    })
                } else {
                    None
                };
                emit(
                    sink,
                    id,
                    &Frame::HashEntry {
                        rel: relative,
                        is_dir: false,
                        size: child.size,
                        mtime_ms: child.mtime_ms,
                        md5,
                    },
                )?;
            }
        }
    }
    emit(sink, id, &Frame::End)
}

fn md5_backend(backend: &BackendHandle, path: &str, cancel: &AtomicBool) -> io::Result<String> {
    let mut reader = backend.open_read(path)?;
    let mut context = md5::Context::new();
    let mut buffer = vec![0u8; CHUNK];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(canceled("daemon backend hash"));
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        context.consume(&buffer[..read]);
    }
    Ok(format!("{:x}", context.compute()))
}

fn require_plain_directory(backend: &BackendHandle, path: &str) -> io::Result<()> {
    let metadata = backend.stat(path)?;
    if metadata.is_symlink || !metadata.is_dir {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("backend walk directory changed type: {path}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
}
