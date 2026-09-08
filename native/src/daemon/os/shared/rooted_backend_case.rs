use crate::vfs::{BackendHandle, CachingBackend, VfsMeta};
use std::io;

/// Resolves one helper-owned virtual path below its authorized backend root.
/// Backends without a proven case-sensitive contract are made predictably
/// case-insensitive here: each component must resolve to at most one physical
/// child, and the backend's preserved spelling is used for every operation.
pub(super) struct PathResolver<'a> {
    pub backend: &'a BackendHandle,
    pub case_cache: Option<&'a CachingBackend>,
    pub root_validator: Option<&'a BackendHandle>,
    pub root: &'a str,
    pub root_ancestors: &'a [String],
    pub case_sensitive: bool,
}

impl PathResolver<'_> {
    pub fn resolve(&self, components: &[String], allow_missing: bool) -> io::Result<String> {
        self.walk(components, allow_missing, None).map(|(path, _)| path)
    }

    /// One fresh terminal observation supplies both link validation and the
    /// returned metadata. Parent-listing metadata is never a fresh stat result.
    pub fn stat(&self, components: &[String], raw: &BackendHandle) -> io::Result<VfsMeta> {
        let (_, metadata) = self.walk(components, false, Some(raw))?;
        metadata.ok_or_else(|| io::Error::other("resolved stat has no terminal metadata"))
    }

    fn walk(&self, components: &[String], allow_missing: bool,
        terminal: Option<&BackendHandle>) -> io::Result<(String, Option<VfsMeta>)> {
        let root_metadata = self.root_validator
            .map(|validator| validate_root(validator, self.root_ancestors)).transpose()?;
        let mut current = self.root.to_string();
        if components.is_empty() {
            let metadata = match terminal {
                Some(raw) => {
                    let metadata = match root_metadata {
                        Some(metadata) => metadata,
                        None => raw.stat(&current)?,
                    };
                    validate_entry(&metadata, false)?;
                    Some(metadata)
                }
                None => None,
            };
            return Ok((current, metadata));
        }
        let mut missing = false;
        for (index, requested) in components.iter().enumerate() {
            if missing {
                append_component(&mut current, requested);
                continue;
            }
            let is_final = index + 1 == components.len();
            let listed_metadata = if self.case_sensitive {
                append_component(&mut current, requested);
                None
            } else {
                match unique_child(self.backend, self.case_cache, &current, requested)? {
                    Some(metadata) => {
                        append_component(&mut current, &metadata.name);
                        Some(metadata)
                    }
                    None if allow_missing => {
                        missing = true;
                        append_component(&mut current, requested);
                        continue;
                    }
                    None => return Err(not_found()),
                }
            };
            if let Some(metadata) = &listed_metadata {
                validate_entry(metadata, is_final)?;
            }
            if let Some(raw) = terminal.filter(|_| is_final) {
                let metadata = match raw.stat(&current) {
                    Ok(metadata) => metadata,
                    // Keep the previous cold case-sensitive error contract,
                    // but do not add a probe or second stat to the success path.
                    Err(error) if listed_metadata.is_none() => {
                        return Err(match raw.try_exists(&current) {
                            Ok(false) => not_found(),
                            Ok(true) => error,
                            Err(probe) => probe,
                        });
                    }
                    Err(error) => return Err(error),
                };
                validate_entry(&metadata, true)?;
                return Ok((current, Some(metadata)));
            }
            if listed_metadata.is_none() {
                match self.backend.stat(&current) {
                    Ok(metadata) => validate_entry(&metadata, is_final)?,
                    Err(stat_error) => match self.backend.try_exists(&current) {
                        Ok(false) if allow_missing => missing = true,
                        Ok(false) => return Err(not_found()),
                        Ok(true) => return Err(stat_error),
                        Err(probe_error) => return Err(probe_error),
                    },
                }
            }
        }
        Ok((current, None))
    }
}

pub(super) fn validate_root(
    backend: &BackendHandle,
    ancestors: &[String],
) -> io::Result<crate::vfs::VfsMeta> {
    let mut root_metadata = None;
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
        root_metadata = Some(metadata);
    }
    root_metadata.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "mount root validation requires at least one path",
        )
    })
}

fn unique_child(
    backend: &BackendHandle,
    case_cache: Option<&CachingBackend>,
    parent: &str,
    requested: &str,
) -> io::Result<Option<crate::vfs::VfsMeta>> {
    if let Some(case_cache) = case_cache {
        let matched = case_cache.unique_child(parent, requested)?;
        if let Some(metadata) = matched.as_ref() {
            validate_child_name(&metadata.name)?;
        }
        return Ok(matched);
    }
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

fn append_component(path: &mut String, child: &str) {
    // Reuse the growing allocation; constructing a new joined prefix at every
    // component copies a deep path's earlier bytes repeatedly.
    let length = path.trim_end_matches('/').len();
    path.truncate(length);
    path.push('/');
    path.push_str(child);
}

fn not_found() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "mount path does not exist")
}

fn permission_denied(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}
