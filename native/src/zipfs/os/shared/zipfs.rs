//! Read-only ZIP archive exposed as a `vfs::Backend`, so a `.zip` opens and
//! browses inside the explorer like a (remote) folder — reusing the whole
//! remote-browse path (rscan walk, open-file-to-temp, the connection indicator).
//! Plus `extract_all` for the "Entpacken" action. Pure-Rust deflate (flate2 /
//! miniz_oxide), so no C deps for the windows-gnu build.
//!
//! The archive is parsed once on open into a directory map; file bytes are read
//! (decompressed) on demand by re-opening the archive. Mutations are
//! unsupported (read-only) — extract to edit.

use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
use std::collections::HashMap;
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};

fn zip_err<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

fn native(path: &str) -> PathBuf {
    PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR))
}

/// Internal zip entry timestamp → unix milliseconds (0 if unrepresentable).
fn dt_ms(dt: zip::DateTime) -> i64 {
    chrono::NaiveDate::from_ymd_opt(dt.year() as i32, dt.month() as u32, dt.day() as u32)
        .and_then(|d| d.and_hms_opt(dt.hour() as u32, dt.minute() as u32, dt.second() as u32))
        .map(|ndt| ndt.and_utc().timestamp_millis())
        .unwrap_or(0)
}

pub struct ZipBackend {
    /// Local path of the `.zip` (re-opened per `open_read`).
    zip_path: String,
    /// Forward-slash display root (the archive path).
    display: String,
    /// Zip-internal dir path (no leading/trailing slash; "" = root) → children.
    dirs: HashMap<String, Vec<VfsMeta>>,
}

fn join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", parent, name)
    }
}

fn add_child(
    dirs: &mut HashMap<String, Vec<VfsMeta>>,
    parent: &str,
    name: &str,
    is_dir: bool,
    size: u64,
    mtime_ms: i64,
) {
    let v = dirs.entry(parent.to_string()).or_default();
    if v.iter().any(|m| m.name == name && m.is_dir == is_dir) {
        return; // already registered (e.g. a dir seen via an explicit entry + a child)
    }
    v.push(VfsMeta {
        name: name.to_string(),
        is_dir,
        is_symlink: false,
        size,
        mtime_ms,
        btime_ms: 0,
        hidden: name.starts_with('.'),
        system: false,
        id: None,
        content_md5: None,
    });
}

/// Register an entry's full internal path, creating any missing ancestor dirs.
fn register(
    dirs: &mut HashMap<String, Vec<VfsMeta>>,
    path: &str,
    is_dir: bool,
    size: u64,
    mtime_ms: i64,
) {
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return;
    }
    let mut cur = String::new();
    for seg in &segs[..segs.len() - 1] {
        add_child(dirs, &cur, seg, true, 0, 0);
        cur = join(&cur, seg);
        dirs.entry(cur.clone()).or_default();
    }
    let leaf = segs[segs.len() - 1];
    add_child(dirs, &cur, leaf, is_dir, size, mtime_ms);
    if is_dir {
        dirs.entry(join(&cur, leaf)).or_default();
    }
}

impl ZipBackend {
    pub fn open(zip_path: &str) -> io::Result<ZipBackend> {
        let f = std::fs::File::open(native(zip_path))?;
        let mut ar = zip::ZipArchive::new(f).map_err(zip_err)?;
        let mut dirs: HashMap<String, Vec<VfsMeta>> = HashMap::new();
        dirs.entry(String::new()).or_default(); // root always exists
        for i in 0..ar.len() {
            let e = ar.by_index(i).map_err(zip_err)?;
            let raw = e.name().trim_end_matches('/').to_string();
            if raw.is_empty() {
                continue;
            }
            let mtime = e.last_modified().map(dt_ms).unwrap_or(0);
            register(&mut dirs, &raw, e.is_dir(), e.size(), mtime);
        }
        for v in dirs.values_mut() {
            v.sort_by(|a, b| {
                // Dirs first, then by name — a sensible default for an archive.
                b.is_dir
                    .cmp(&a.is_dir)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
        }
        Ok(ZipBackend {
            zip_path: zip_path.to_string(),
            display: zip_path.to_string(),
            dirs,
        })
    }

    fn key(path: &str) -> String {
        path.trim_start_matches('/')
            .trim_end_matches('/')
            .to_string()
    }
}

impl Backend for ZipBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Local
    }
    fn root_display(&self) -> String {
        self.display.clone()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        match self.dirs.get(&Self::key(path)) {
            Some(v) => Ok(v.clone()),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Ordner nicht im Archiv",
            )),
        }
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let key = Self::key(path);
        if key.is_empty() || self.dirs.contains_key(&key) {
            return Ok(VfsMeta {
                name: key.rsplit('/').next().unwrap_or("").to_string(),
                is_dir: true,
                ..Default::default()
            });
        }
        let (parent, name) = key.rsplit_once('/').unwrap_or(("", key.as_str()));
        self.dirs
            .get(parent)
            .and_then(|v| v.iter().find(|m| m.name == name))
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "nicht im Archiv"))
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let name = path.trim_start_matches('/');
        let f = std::fs::File::open(native(&self.zip_path))?;
        let mut ar = zip::ZipArchive::new(f).map_err(zip_err)?;
        let mut zf = ar.by_name(name).map_err(zip_err)?;
        let mut buf = Vec::with_capacity(zf.size() as usize);
        zf.read_to_end(&mut buf)?;
        Ok(Box::new(Cursor::new(buf)))
    }

    // Read-only archive: all mutations are unsupported.
    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Err(readonly())
    }
    fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
        Err(readonly())
    }
    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        Err(readonly())
    }
    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Err(readonly())
    }
    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Err(readonly())
    }
    fn parallelism(&self) -> usize {
        1 // single archive file; no benefit to parallel listing
    }
}

fn readonly() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "ZIP ist schreibgeschützt — zum Bearbeiten entpacken",
    )
}

/// Extract the whole archive into `dest` (created if missing). Uses
/// `enclosed_name` so a malicious archive can't escape `dest` (zip-slip safe).
/// Returns the number of files written.
pub fn extract_all(zip_path: &str, dest: &Path) -> io::Result<usize> {
    let f = std::fs::File::open(native(zip_path))?;
    let mut ar = zip::ZipArchive::new(f).map_err(zip_err)?;
    std::fs::create_dir_all(dest)?;
    let mut count = 0usize;
    for i in 0..ar.len() {
        let mut e = ar.by_index(i).map_err(zip_err)?;
        let rel = match e.enclosed_name() {
            Some(p) => p,
            None => continue, // unsafe / absolute path — skip
        };
        let out = dest.join(rel);
        if e.is_dir() {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(p) = out.parent() {
                std::fs::create_dir_all(p)?;
            }
            let mut w = std::fs::File::create(&out)?;
            io::copy(&mut e, &mut w)?;
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_zip(path: &Path) {
        let f = std::fs::File::create(path).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let opt = zip::write::SimpleFileOptions::default();
        w.start_file("a.txt", opt).unwrap();
        w.write_all(b"hello").unwrap();
        w.add_directory("sub", opt).unwrap();
        w.start_file("sub/b.bin", opt).unwrap();
        w.write_all(&[0u8; 250]).unwrap();
        w.finish().unwrap();
    }

    #[test]
    fn browse_and_extract() {
        let base = std::env::temp_dir().join(format!("se_zip_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let zip_path = base.join("arc.zip");
        make_zip(&zip_path);
        let zp = zip_path.to_string_lossy().replace('\\', "/");

        let be = ZipBackend::open(&zp).unwrap();
        // Root listing: a.txt + sub/
        let mut root = be.list_dir("/").unwrap();
        root.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(root.len(), 2);
        assert_eq!(root.iter().find(|m| m.name == "a.txt").unwrap().size, 5);
        assert!(root.iter().find(|m| m.name == "sub").unwrap().is_dir);
        // Nested listing + read.
        let sub = be.list_dir("/sub").unwrap();
        assert_eq!(sub.iter().find(|m| m.name == "b.bin").unwrap().size, 250);
        let mut buf = Vec::new();
        be.open_read("/a.txt")
            .unwrap()
            .read_to_end(&mut buf)
            .unwrap();
        assert_eq!(buf, b"hello");
        // Read-only.
        assert!(be.open_write("/x").is_err());

        // Extract.
        let out = base.join("out");
        let n = extract_all(&zp, &out).unwrap();
        assert_eq!(n, 2);
        assert_eq!(std::fs::read(out.join("a.txt")).unwrap(), b"hello");
        assert_eq!(std::fs::read(out.join("sub/b.bin")).unwrap().len(), 250);

        let _ = std::fs::remove_dir_all(&base);
    }
}
