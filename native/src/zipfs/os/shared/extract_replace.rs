//! Whole-archive extraction that replaces existing regular files (the
//! "Entpacken" action on a remote ZIP) without ever writing through a link:
//! an archive link entry is skipped, and so is an entry whose target path
//! already passes over a link (symlink; on Windows also a junction or any
//! other redirecting reparse point, which `FileType::is_symlink` reports) below
//! the destination folder. The destination folder itself may be a link.
use super::extract_lanes::Job;
use super::names::enclosed_name;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Result of one extraction.
#[derive(Debug, Default)]
pub(super) struct ReplaceReport {
    pub(super) files: usize,
    /// Entries not written: `(archive path, reason)`.
    pub(super) skipped: Vec<(String, String)>,
}

/// The first existing component of `relative` below `dest` that is a link.
/// Components that do not exist yet end the walk: nothing below them can be a
/// link, and they are created as plain folders by the extraction itself.
fn linked_component(dest: &Path, relative: &Path) -> io::Result<Option<PathBuf>> {
    let mut current = dest.to_path_buf();
    for part in relative.components() {
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return Ok(Some(current)),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

fn zip_error<E: std::fmt::Display>(error: E) -> io::Error {
    io::Error::other(error.to_string())
}

/// Extracts every entry of `zip_path` below `dest` (created if missing).
/// Names, links and folders are decided in archive order on this thread;
/// the files are then written by parallel lanes (`extract_lanes`). The
/// first failure (lowest archive index) ends the extraction, as before.
pub(super) fn extract_replacing(zip_path: &Path, dest: &Path) -> io::Result<ReplaceReport> {
    let archive = super::extract_lanes::open(zip_path)?;
    std::fs::create_dir_all(dest)?;
    let mut report = ReplaceReport::default();
    let mut skipped = Vec::new();
    let mut jobs = Vec::new();
    {
        let mut archive = archive.clone();
        for index in 0..archive.len() {
            let entry = archive.by_index_raw(index).map_err(zip_error)?;
            let Some(relative) = enclosed_name(&entry) else {
                continue; // unsafe / absolute path: skipped silently as before
            };
            let name = relative.to_string_lossy().replace('\\', "/");
            if entry.is_symlink() {
                skipped.push((index, name, "Links werden nicht entpackt".to_string()));
                continue;
            }
            if let Some(link) = linked_component(dest, &relative)? {
                skipped.push((
                    index,
                    name,
                    format!("Ziel liegt hinter einem Link: {}", link.display()),
                ));
                continue;
            }
            let out = dest.join(&relative);
            if entry.is_dir() {
                std::fs::create_dir_all(&out)?;
            } else {
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                jobs.push(Job {
                    index,
                    name,
                    target: out,
                });
            }
        }
    }
    let never = AtomicBool::new(false);
    let failed = AtomicBool::new(false);
    let outcomes: Vec<io::Result<usize>> = std::thread::scope(|scope| {
        let workers: Vec<_> = super::extract_lanes::lanes(jobs)
            .into_iter()
            .map(|lane| {
                let mut archive = archive.clone();
                let (never, failed) = (&never, &failed);
                scope.spawn(move || -> io::Result<usize> {
                    let mut files = 0;
                    for job in lane {
                        if failed.load(Ordering::Acquire) {
                            break;
                        }
                        let written = (|| {
                            let mut entry = archive.by_index(job.index).map_err(zip_error)?;
                            let mut writer = std::fs::File::create(&job.target)?;
                            super::extract_lanes::copy(&mut entry, &mut writer, never, |_| {})
                        })();
                        if let Err(error) = written {
                            failed.store(true, Ordering::Release);
                            return Err(io::Error::new(
                                error.kind(),
                                IndexedError(job.index, error.to_string()),
                            ));
                        }
                        files += 1;
                    }
                    Ok(files)
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|_| Err(io::Error::other("Entpacken unerwartet beendet")))
            })
            .collect()
    });
    let mut first: Option<(usize, io::Error)> = None;
    for outcome in outcomes {
        match outcome {
            Ok(files) => report.files += files,
            Err(error) => {
                let index = error
                    .get_ref()
                    .and_then(|inner| inner.downcast_ref::<IndexedError>())
                    .map_or(usize::MAX, |indexed| indexed.0);
                if first.as_ref().is_none_or(|(known, _)| index < *known) {
                    first = Some((index, error));
                }
            }
        }
    }
    if let Some((_, error)) = first {
        return Err(error);
    }
    skipped.sort_by_key(|(index, _, _)| *index);
    report.skipped = skipped
        .into_iter()
        .map(|(_, name, reason)| (name, reason))
        .collect();
    Ok(report)
}

/// A lane failure with the archive index it happened at; its text is the
/// original error's text.
#[derive(Debug)]
struct IndexedError(usize, String);

impl std::fmt::Display for IndexedError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(&self.1)
    }
}

impl std::error::Error for IndexedError {}

#[cfg(test)]
pub(super) mod test_support {
    use std::io::Write;
    use std::path::{Path, PathBuf};

    /// A fresh empty folder below the temp directory.
    pub(in crate::zipfs) fn temp_base(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("se_zip_{tag}_{}_{nanos}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    /// Writes an archive of regular `files` and link entries `(name, target)`.
    pub(in crate::zipfs) fn write_zip(
        path: &Path,
        files: &[(&str, &[u8])],
        links: &[(&str, &str)],
    ) {
        let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for (name, data) in files {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        for (name, target) in links {
            writer.add_symlink(*name, *target, options).unwrap();
        }
        writer.finish().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{temp_base, write_zip};
    use crate::zipfs::extract_all;

    #[test]
    fn zipfs_extract_skips_link_entries() {
        let base = temp_base("links");
        let zip_path = base.join("arc.zip");
        write_zip(
            &zip_path,
            &[("safe.txt", b"ok")],
            &[("link.txt", "/etc/passwd")],
        );
        let zp = zip_path.to_string_lossy().replace('\\', "/");
        let out = base.join("out");

        let report = super::extract_replacing(&zip_path, &out).unwrap();
        assert_eq!(report.files, 1);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].0, "link.txt");
        assert_eq!(report.skipped[0].1, "Links werden nicht entpackt");
        assert!(!out.join("link.txt").exists());
        assert!(std::fs::symlink_metadata(out.join("link.txt")).is_err());
        assert_eq!(std::fs::read(out.join("safe.txt")).unwrap(), b"ok");
        // The public entry point counts only the written file.
        assert_eq!(extract_all(&zp, &base.join("out2")).unwrap(), 1);
        assert!(!base.join("out2").join("link.txt").exists());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn zipfs_extract_replaces_an_existing_regular_file() {
        let base = temp_base("replace");
        let zip_path = base.join("arc.zip");
        write_zip(
            &zip_path,
            &[("sub/a.txt", b"new"), ("b.txt", b"fresh")],
            &[],
        );
        let zp = zip_path.to_string_lossy().replace('\\', "/");
        let out = base.join("out");
        std::fs::create_dir_all(out.join("sub")).unwrap();
        std::fs::write(out.join("sub/a.txt"), b"old content that is longer").unwrap();

        assert_eq!(extract_all(&zp, &out).unwrap(), 2);
        assert_eq!(std::fs::read(out.join("sub/a.txt")).unwrap(), b"new");
        assert_eq!(std::fs::read(out.join("b.txt")).unwrap(), b"fresh");

        let _ = std::fs::remove_dir_all(&base);
    }
}
