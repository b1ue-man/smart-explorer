//! Directory listing sources for the discovery walker: any `vfs::Backend`, or
//! the local filesystem through `local_access`.
use crate::vfs::{Backend, VfsMeta};
use std::io;
use std::path::PathBuf;

/// One child entry as the walker needs it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Listed {
    pub name: String,
    pub is_dir: bool,
    pub is_link: bool,
    pub size: u64,
    pub mtime_ms: i64,
    pub btime_ms: i64,
    pub hidden: bool,
    pub system: bool,
    pub id: Option<String>,
    /// Content MD5 the provider lists for free (Drive, Nextcloud).
    pub md5: Option<String>,
    /// The entry exists but cannot be addressed (for example a name that is
    /// not valid Unicode); it is reported, not transferred.
    pub problem: Option<String>,
}

impl From<VfsMeta> for Listed {
    fn from(meta: VfsMeta) -> Self {
        Self {
            name: meta.name,
            is_dir: meta.is_dir && !meta.is_symlink,
            is_link: meta.is_symlink,
            size: meta.size,
            mtime_ms: meta.mtime_ms,
            btime_ms: meta.btime_ms,
            hidden: meta.hidden,
            system: meta.system,
            id: meta.id,
            md5: meta.content_md5,
            problem: None,
        }
    }
}

/// Lists folders and inspects single entries of one namespace.
pub(crate) trait Lister: Sync {
    fn list(&self, dir: &str) -> io::Result<Vec<Listed>>;
    fn stat(&self, path: &str) -> io::Result<Listed>;
}

pub(crate) struct BackendLister<'a>(pub &'a dyn Backend);

impl Lister for BackendLister<'_> {
    fn list(&self, dir: &str) -> io::Result<Vec<Listed>> {
        Ok(self
            .0
            .list_dir(dir)?
            .into_iter()
            .map(Listed::from)
            .collect())
    }

    fn stat(&self, path: &str) -> io::Result<Listed> {
        self.0.stat(path).map(Listed::from)
    }
}

/// The local filesystem through `local_access` (verbatim paths for Win32-
/// hostile names). Permission refusals are returned as they are; the walker
/// asks the transfer's `AccessGate` outside of any flow permit.
pub(crate) struct LocalLister;

pub(crate) fn native(path: &str) -> PathBuf {
    PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR))
}

impl Lister for LocalLister {
    fn list(&self, dir: &str) -> io::Result<Vec<Listed>> {
        let path = native(dir);
        {
            let mut listed = Vec::new();
            for entry in crate::local_access::read_directory(&path)? {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error),
                };
                let is_link =
                    entry.is_link_like || entry.kind == crate::local_access::EntryKind::Link;
                let special = entry.kind == crate::local_access::EntryKind::Other && !entry.is_dir;
                let (name, mut problem) = match entry.name.into_string() {
                    Ok(name) if !entry.unreachable => (name, None),
                    Ok(name) => (
                        name,
                        Some("Name ist kein darstellbarer Dateipfad".to_string()),
                    ),
                    Err(raw) => (
                        raw.to_string_lossy().into_owned(),
                        Some("Dateiname ist kein gültiges Unicode".to_string()),
                    ),
                };
                if special && problem.is_none() {
                    problem = Some("Spezialdatei wird nicht übertragen".to_string());
                }
                listed.push(Listed {
                    name,
                    is_dir: entry.is_dir && !is_link,
                    is_link,
                    size: if entry.is_dir { 0 } else { entry.size },
                    mtime_ms: entry.mtime_ms,
                    btime_ms: entry.btime_ms,
                    hidden: entry.hidden,
                    system: entry.system,
                    id: None,
                    md5: None,
                    problem,
                });
            }
            Ok(listed)
        }
    }

    fn stat(&self, path: &str) -> io::Result<Listed> {
        let native_path = native(path);
        {
            let metadata = crate::local_access::symlink_metadata(&native_path)?;
            let is_link = crate::local_access::metadata_is_link_like(&native_path, &metadata);
            let name = path
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_string();
            Ok(Listed {
                name,
                is_dir: metadata.is_dir() && !is_link,
                is_link: is_link || !(metadata.is_dir() || metadata.is_file()),
                size: if metadata.is_dir() { 0 } else { metadata.len() },
                mtime_ms: metadata
                    .modified()
                    .map(crate::local_access::system_time_ms)
                    .unwrap_or(0),
                btime_ms: metadata
                    .created()
                    .map(crate::local_access::system_time_ms)
                    .unwrap_or(0),
                hidden: false,
                system: false,
                id: None,
                md5: None,
                problem: None,
            })
        }
    }
}
