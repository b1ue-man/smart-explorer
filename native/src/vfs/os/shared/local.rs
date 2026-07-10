use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use super::local_platform;
use super::{Backend, Scheme, VfsMeta, VfsResult};

// Intentionally duplicated from `scanner.rs` (tiny) to keep this module
// self-contained - isolation over DRY, per the staged remote-layer plan.

#[inline]
fn ms_since_unix(t: std::time::SystemTime) -> i64 {
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

fn meta_to_vfs(name: String, meta: &std::fs::Metadata) -> VfsMeta {
    let (hidden, system) = local_platform::local_attrs(meta);
    let is_symlink = meta.is_symlink() || local_platform::is_reparse_point(meta);
    let is_dir = meta.is_dir() && !is_symlink;
    VfsMeta {
        name,
        is_dir,
        is_symlink,
        size: if is_dir { 0 } else { meta.len() },
        mtime_ms: meta.modified().map(ms_since_unix).unwrap_or(0),
        btime_ms: meta.created().map(ms_since_unix).unwrap_or(0),
        hidden,
        system,
        id: None,
        content_md5: None,
    }
}

fn unicode_name(name: &OsStr) -> io::Result<String> {
    name.to_str().map(str::to_owned).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "local filename is not valid Unicode",
        )
    })
}

/// `std::fs`-backed local disk using the host path adapter at the boundary.
pub struct LocalBackend {
    root: String, // forward-slash, trailing slash trimmed (display only)
}

impl LocalBackend {
    pub fn new(root: &str) -> Self {
        let r = root.trim().replace('\\', "/");
        let r = r.trim_end_matches('/');
        LocalBackend {
            root: if r.is_empty() {
                "/".to_string()
            } else {
                r.to_string()
            },
        }
    }
}

impl Backend for LocalBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Local
    }
    fn root_display(&self) -> String {
        self.root.clone()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let dir = local_platform::to_os(path);
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = match entry {
                Ok(entry) => entry,
                // Directory contents can change after read_dir captured its
                // cursor. A vanished unrelated child must not make the whole
                // listing fail; callers still revalidate every expected child.
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            let name = unicode_name(&entry.file_name())?;
            let meta = match std::fs::symlink_metadata(entry.path()) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            out.push(meta_to_vfs(name, &meta));
        }
        Ok(out)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let p = local_platform::to_os(path);
        let meta = std::fs::symlink_metadata(&p)?;
        let name = p
            .file_name()
            .map(unicode_name)
            .transpose()?
            .unwrap_or_else(|| path.to_string());
        Ok(meta_to_vfs(name, &meta))
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        Ok(Box::new(std::fs::File::open(local_platform::to_os(path))?))
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(std::fs::File::create(local_platform::to_os(
            path,
        ))?))
    }
    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        let staged = super::promotion::unique_staging_path(self, dst, "copy")?;
        let result = (|| {
            let copied = std::fs::copy(local_platform::to_os(src), local_platform::to_os(&staged))?;
            super::promotion::promote_staged_replace(self, &staged, dst)?;
            Ok(copied)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(local_platform::to_os(&staged));
        }
        result
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        std::fs::rename(local_platform::to_os(src), local_platform::to_os(dst))
    }
    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        local_platform::rename_no_replace(&local_platform::to_os(src), &local_platform::to_os(dst))
    }
    fn rename_overwrites(&self) -> bool {
        true // std::fs::rename atomically replaces an existing destination
    }
    fn is_local(&self) -> bool {
        true // a local disk read to hash a file is cheap (no network)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        local_platform::remove_file_like(&local_platform::to_os(path))
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        std::fs::remove_dir(local_platform::to_os(path))
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        mkdir_all_plain(&local_platform::to_os(path))
    }
}

/// Create each missing component while refusing existing symlinks, junctions,
/// and other reparse points. `std::fs::create_dir_all` follows such ancestors,
/// which can redirect a selected sync/copy root outside its authorized tree.
fn mkdir_all_plain(path: &Path) -> io::Result<()> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut current = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(std::path::MAIN_SEPARATOR_STR),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "directory creation contains a parent component",
                ))
            }
            Component::Normal(name) => {
                current.push(name);
                ensure_plain_component(&current)?;
            }
        }
    }
    Ok(())
}

fn ensure_plain_component(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => validate_plain_component(path, &metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => match std::fs::create_dir(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                validate_plain_component(path, &std::fs::symlink_metadata(path)?)
            }
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    }
}

fn validate_plain_component(path: &Path, metadata: &std::fs::Metadata) -> io::Result<()> {
    if metadata.is_symlink() || local_platform::is_reparse_point(metadata) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "directory ancestor is a link or reparse point: {}",
                path.display()
            ),
        ));
    }
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("directory ancestor is not a directory: {}", path.display()),
        ));
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod mkdir_tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn mkdir_all_rejects_link_ancestor() {
        let base = std::env::temp_dir().join(format!(
            "se-vfs-mkdir-link-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let victim = base.join("victim");
        let link = base.join("link");
        std::fs::create_dir_all(&victim).unwrap();
        symlink(&victim, &link).unwrap();
        assert!(mkdir_all_plain(&link.join("child")).is_err());
        assert!(!victim.join("child").exists());
        std::fs::remove_file(link).ok();
        std::fs::remove_dir_all(base).ok();
    }
}
