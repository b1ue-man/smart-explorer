//! Program lookup of grants with a program list.
//!
//! A program is found only as an absolute path or in the fixed search path of
//! the host platform (`ProgramSearch`, supplied by the OS adapter). The
//! caller's own `PATH` never takes part: the list would otherwise be bypassed
//! by pointing `PATH` at a file the caller can write. A start is permitted
//! when the file it resolves to is the file a list entry resolves to, under
//! the same file name; the same name keeps multi-call binaries (busybox,
//! toybox: `ls` and `sh` are one file) from turning a listed applet into any
//! other applet of that binary.
use std::path::{is_separator, Path, PathBuf};

/// Where bare program names are looked up.
#[derive(Clone, Debug)]
pub(crate) struct ProgramSearch {
    pub(crate) directories: Vec<PathBuf>,
    /// Tried after the bare name when it has no extension (`.exe`).
    pub(crate) suffixes: &'static [&'static str],
}

/// A program file as found, and its identity behind links.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedProgram {
    /// The path the job is started with; it keeps the program's own name.
    pub(crate) path: PathBuf,
    canonical: PathBuf,
}

/// Looks `program` up without consulting any environment variable.
pub(crate) fn resolve(program: &str, search: &ProgramSearch) -> Result<ResolvedProgram, String> {
    let direct = Path::new(program);
    let candidates: Vec<PathBuf> = if direct.is_absolute() {
        variants(direct, search)
    } else if program.chars().any(is_separator) {
        return Err(format!(
            "program {program:?} names a directory but is not an absolute path"
        ));
    } else {
        search
            .directories
            .iter()
            .flat_map(|directory| variants(&directory.join(program), search))
            .collect()
    };
    for candidate in candidates {
        let is_file = std::fs::metadata(&candidate).is_ok_and(|metadata| metadata.is_file());
        if !is_file {
            continue;
        }
        if let Ok(canonical) = std::fs::canonicalize(&candidate) {
            return Ok(ResolvedProgram {
                path: candidate,
                canonical,
            });
        }
    }
    Err(format!(
        "program {program:?} is not an absolute path to a file and not in this host's fixed search path"
    ))
}

/// The resolved path `program` may start with, or why it may not.
pub(crate) fn authorize(
    program: &str,
    listed: &[String],
    search: &ProgramSearch,
) -> Result<PathBuf, String> {
    let requested = resolve(program, search)?;
    let permitted = listed.iter().any(|entry| {
        resolve(entry, search).is_ok_and(|entry| {
            entry.canonical == requested.canonical && same_name(&entry.path, &requested.path)
        })
    });
    if permitted {
        Ok(requested.path)
    } else {
        Err(format!(
            "this host allows this device only its listed programs; {program:?} is not one of them"
        ))
    }
}

fn variants(path: &Path, search: &ProgramSearch) -> Vec<PathBuf> {
    let mut paths = vec![path.to_path_buf()];
    if path.extension().is_none() {
        for suffix in search.suffixes {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            paths.push(PathBuf::from(name));
        }
    }
    paths
}

fn same_name(left: &Path, right: &Path) -> bool {
    match (left.file_name(), right.file_name()) {
        (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(directory: &Path, suffixes: &'static [&'static str]) -> ProgramSearch {
        ProgramSearch {
            directories: vec![directory.to_path_buf()],
            suffixes,
        }
    }

    #[test]
    fn luecken_task_a_bare_name_is_completed_with_the_platform_suffix_only() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("tool.exe"), b"x").unwrap();
        std::fs::write(directory.path().join("script.cmd"), b"x").unwrap();
        let windows_like = search(directory.path(), &[".exe", ".com"]);
        let found = resolve("tool", &windows_like).unwrap();
        assert_eq!(found.path, directory.path().join("tool.exe"));
        assert_eq!(
            resolve("TOOL.EXE", &windows_like).is_ok(),
            cfg!(windows),
            "file name case follows the file system"
        );
        // A command script is never completed: it would start a shell.
        assert!(resolve("script", &windows_like).is_err());
        // Without suffixes the name must be exact.
        assert!(resolve("tool", &search(directory.path(), &[])).is_err());
    }

    #[test]
    fn luecken_task_a_listed_program_matches_by_bare_name_or_absolute_path() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("Tool"), b"x").unwrap();
        let search = search(directory.path(), &[]);
        let listed = vec!["Tool".to_string()];
        assert!(authorize("Tool", &listed, &search).is_ok());
        let absolute = directory.path().join("Tool");
        let listed_absolute = vec![absolute.to_string_lossy().into_owned()];
        assert!(authorize("Tool", &listed_absolute, &search).is_ok());
        // A listed program that does not exist grants nothing.
        let missing = vec!["Missing".to_string()];
        assert!(authorize("Tool", &missing, &search).is_err());
    }
}
