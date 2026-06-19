use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use super::core_oslocked::{is_pseudo_dir, systemtime_ms};
use super::session::{emit, Sink};
use super::{Frame, SearchSpec};

fn glob_match(pat: &str, s: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (
        pat.to_lowercase().chars().collect(),
        s.to_lowercase().chars().collect(),
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

fn matches_spec(name: &str, is_dir: bool, size: u64, spec: &SearchSpec) -> bool {
    if is_dir && !spec.want_dirs {
        return false;
    }
    if !is_dir {
        if size < spec.min_size {
            return false;
        }
        if spec.max_size != 0 && size > spec.max_size {
            return false;
        }
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

/// Recursive server-side search -> stream `Match` per hit, then `End`.
pub(crate) fn handle_search(
    sink: &Sink,
    id: u64,
    root: &str,
    spec: &SearchSpec,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let base = Path::new(root);
    let mut count = 0u64;
    let mut stack = vec![base.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for ent in rd.flatten() {
            let ft = match ent.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_symlink() {
                continue;
            }
            let p = ent.path();
            let nm = ent.file_name().to_string_lossy().into_owned();
            let md = ent.metadata().ok();
            let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
            let mtime = md
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(systemtime_ms)
                .unwrap_or(0);
            if ft.is_dir() {
                if is_pseudo_dir(&p.to_string_lossy()) {
                    continue;
                }
                stack.push(p.clone());
            }
            if matches_spec(&nm, ft.is_dir(), size, spec) {
                let rel = p
                    .strip_prefix(base)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                emit(
                    sink,
                    id,
                    &Frame::Match {
                        rel,
                        is_dir: ft.is_dir(),
                        size,
                        mtime_ms: mtime,
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

#[cfg(test)]
mod tests {
    use super::glob_match;

    #[test]
    fn glob_matches() {
        assert!(glob_match("*.txt", "a.txt"));
        assert!(glob_match("foo?", "foob"));
        assert!(!glob_match("*.txt", "a.bin"));
        assert!(glob_match("*report*", "Q3_Report_final"));
    }
}
