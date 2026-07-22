use crate::vfs::BackendHandle;
use std::io;

/// Resolves one helper-owned virtual path below its authorized backend root.
/// Backends without a proven case-sensitive contract are made predictably
/// case-insensitive here: each component must resolve to at most one physical
/// child, and the backend's preserved spelling is used for every operation.
pub(super) fn resolve(
    backend: &BackendHandle,
    root: &str,
    root_ancestors: &[String],
    requested_components: &[String],
    allow_missing: bool,
    case_sensitive: bool,
) -> io::Result<String> {
    validate_root(backend, root_ancestors)?;
    let mut current = root.to_string();
    let mut missing = false;
    for (index, requested) in requested_components.iter().enumerate() {
        if missing {
            current = join(&current, requested);
            continue;
        }
        let is_final = index + 1 == requested_components.len();
        let (candidate, listed_metadata) = if case_sensitive {
            (join(&current, requested), None)
        } else {
            match unique_child(backend, &current, requested)? {
                Some(metadata) => {
                    let candidate = join(&current, &metadata.name);
                    (candidate, Some(metadata))
                }
                None if allow_missing => {
                    missing = true;
                    current = join(&current, requested);
                    continue;
                }
                None => return Err(not_found()),
            }
        };
        if let Some(metadata) = listed_metadata {
            // Case-folded lookup already had to list this exact parent. Reuse
            // that entry's type/link facts instead of a second remote stat.
            // Enforced confinement remains the backend's independent contract;
            // trusted-root backends already cannot make this lookup atomic
            // against external namespace races.
            validate_entry(&metadata, is_final)?;
        } else {
            match backend.stat(&candidate) {
                Ok(metadata) => validate_entry(&metadata, is_final)?,
                Err(stat_error) => match backend.try_exists(&candidate) {
                    Ok(false) if allow_missing => missing = true,
                    Ok(false) => return Err(not_found()),
                    Ok(true) => return Err(stat_error),
                    Err(probe_error) => return Err(probe_error),
                },
            }
        }
        current = candidate;
    }
    Ok(current)
}

fn validate_root(backend: &BackendHandle, ancestors: &[String]) -> io::Result<()> {
    for ancestor in ancestors {
        let metadata = backend.stat(ancestor)?;
        if metadata.is_symlink {
            return Err(permission_denied(
                "mount root crosses a link-like backend entry",
            ));
        }
        if !metadata.is_dir {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "mount root ancestor is not a directory",
            ));
        }
    }
    Ok(())
}

fn unique_child(
    backend: &BackendHandle,
    parent: &str,
    requested: &str,
) -> io::Result<Option<crate::vfs::VfsMeta>> {
    let key = crate::mount::windows_ordinal_key(requested);
    let mut matched = None;
    for metadata in backend.list_dir(parent)? {
        if crate::mount::windows_ordinal_key(&metadata.name) != key {
            continue;
        }
        validate_child_name(&metadata.name)?;
        if matched.replace(metadata).is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "backend contains case-colliding child names",
            ));
        }
    }
    Ok(matched)
}

fn validate_entry(metadata: &crate::vfs::VfsMeta, is_final: bool) -> io::Result<()> {
    if metadata.is_symlink {
        return Err(permission_denied(
            "mount path crosses a link-like backend entry",
        ));
    }
    if !is_final && !metadata.is_dir {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            "mount path ancestor is not a directory",
        ));
    }
    Ok(())
}

fn validate_child_name(name: &str) -> io::Result<()> {
    if name.is_empty()
        || matches!(name, "." | "..")
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "backend returned an unsafe child name",
        ))
    } else {
        Ok(())
    }
}

fn join(parent: &str, child: &str) -> String {
    if parent == "/" {
        format!("/{child}")
    } else {
        format!("{}/{child}", parent.trim_end_matches('/'))
    }
}

fn not_found() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "mount path does not exist")
}

fn permission_denied(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}
