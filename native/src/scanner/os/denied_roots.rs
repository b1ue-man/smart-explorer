//! Lossless retry roots and their old subtotals, independent of UI diagnostics.
use std::collections::BTreeMap;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

const MEMORY_RECORDS: usize = 1024;

#[derive(Clone, Default)]
pub struct DeniedRoots(Arc<Mutex<Store>>);

/// Metadata entries visited below a directory; the directory's own entry belongs
/// to its parent walk and is excluded, exactly like a standalone scan root.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanVisits {
    pub scanned: u64,
    pub bytes: u64,
}
impl ScanVisits {
    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        Some(Self {
            scanned: self.scanned.checked_add(rhs.scanned)?,
            bytes: self.bytes.checked_add(rhs.bytes)?,
        })
    }
}
struct Record {
    path: String,
    size: Option<u64>,
    visits: Option<ScanVisits>,
}
struct Store {
    memory: Vec<Record>,
    spill: Option<Spill>,
    warning: Option<String>,
    // A fixed-size negative filter avoids a disk lookup for every directory.
    // Collisions add harmless subtotal records, never omit a denied root.
    filter: [u64; 1024],
}
impl Default for Store {
    fn default() -> Self {
        Self {
            memory: Vec::new(),
            spill: None,
            warning: None,
            filter: [0; 1024],
        }
    }
}
struct Spill {
    file: File,
    _pin: crate::local_access::DirectoryHandle,
    _directory: tempfile::TempDir,
}

/// Local host spelling; literal paths are retained for every actual read.
fn key(path: &str) -> String {
    let path = path.replace('\\', "/");
    let path = if let Some(rest) = path.strip_prefix("//?/UNC/") {
        format!("//{rest}")
    } else {
        path.strip_prefix("//?/").unwrap_or(&path).to_owned()
    };
    #[cfg(windows)]
    let path = path.to_lowercase();
    path.trim_end_matches('/').to_owned()
}
fn filter_bit(path: &str) -> (usize, u64) {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    key(path).hash(&mut hash);
    let bit = (hash.finish() % (1024 * 64)) as usize;
    (bit / 64, 1 << (bit % 64))
}
pub fn retry_contains(root: &str, path: &str) -> bool {
    let root = key(root);
    let path = key(path);
    path == root
        || path
            .strip_prefix(&root)
            .is_some_and(|tail| tail.starts_with('/'))
}

impl Store {
    fn append(&mut self, record: Record) {
        if self.spill.is_none() && self.memory.len() >= MEMORY_RECORDS && self.warning.is_none() {
            match private_spill(&self.memory) {
                Ok(file) => {
                    self.spill = Some(file);
                    self.memory.clear();
                }
                Err(error) => self.warning = Some(error.to_string()),
            }
        }
        if self.warning.is_some() {
            self.memory.push(record);
            return;
        }
        if let Some(spill) = &mut self.spill {
            if let Err(error) = write_record(&mut spill.file, &record) {
                self.warning = Some(error.to_string());
                self.memory.push(record); // Retain the root even on ENOSPC.
            }
        } else {
            self.memory.push(record);
        }
    }
}
impl DeniedRoots {
    pub fn record(&self, path: impl Into<String>) {
        let path = path.into();
        let mut store = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let normalized = key(&path);
        let mut ancestor = normalized.as_str();
        loop {
            let (word, bit) = filter_bit(ancestor);
            store.filter[word] |= bit;
            match ancestor.rsplit_once('/') {
                Some((parent, _)) => ancestor = parent,
                None => break,
            }
        }
        store.append(Record {
            path,
            size: None,
            visits: None,
        });
    }
    pub fn record_size(&self, path: impl Into<String>, size: u64) {
        let path = path.into();
        let mut store = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let (word, bit) = filter_bit(&path);
        if store.filter[word] & bit != 0 {
            store.append(Record {
                path,
                size: Some(size),
                visits: None,
            });
        }
    }
    pub fn record_visits(&self, path: impl Into<String>, visits: ScanVisits) {
        let path = path.into();
        let mut store = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let (word, bit) = filter_bit(&path);
        if store.filter[word] & bit != 0 {
            store.append(Record {
                path,
                size: None,
                visits: Some(visits),
            });
        }
    }
    pub(super) fn record_scan_visits(&self, path: String, visits: ScanVisits) {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .append(Record {
                path,
                size: None,
                visits: Some(visits),
            });
    }
    /// Missing means unmeasured, not an empty subtree.
    pub fn visits(&self, path: &str) -> io::Result<Option<ScanVisits>> {
        self.load()
            .map(|mut records| records.remove(&key(path)).and_then(|record| record.3))
    }
    pub fn copy_visits_to(&self, target: &Self) -> io::Result<()> {
        for (path, _, _, visits) in self.load()?.into_values() {
            if let Some(visits) = visits {
                target.record_visits(path, visits);
            }
        }
        Ok(())
    }
    pub fn storage_warning(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .warning
            .clone()
    }
    pub fn roots(&self) -> io::Result<Vec<String>> {
        self.roots_with_sizes()
            .map(|roots| roots.into_iter().map(|(path, _)| path).collect())
    }
    pub fn measurements(&self) -> io::Result<Vec<(String, u64)>> {
        self.load().map(|records| {
            records
                .into_values()
                .map(|(path, size, _, _)| (path, size))
                .collect()
        })
    }
    pub fn copy_sizes_to(&self, target: &Self) -> io::Result<()> {
        for (path, size) in self.measurements()? {
            target.record_size(path, size);
        }
        Ok(())
    }
    pub fn roots_with_sizes(&self) -> io::Result<Vec<(String, u64)>> {
        let records = self.load()?;
        let roots: BTreeMap<_, _> = records
            .into_iter()
            .filter(|(_, (_, _, denied, _))| *denied)
            .map(|(key, (path, size, _, _))| (key, (path, size)))
            .collect();
        let mut result = Vec::new();
        for (normalized, record) in &roots {
            let mut ancestor = normalized.as_str();
            let mut covered = false;
            while let Some((parent, _)) = ancestor.rsplit_once('/') {
                if roots.contains_key(parent) {
                    covered = true;
                    break;
                }
                ancestor = parent;
            }
            if !covered {
                result.push(record.clone());
            }
        }
        Ok(result)
    }
    fn load(&self) -> io::Result<BTreeMap<String, (String, u64, bool, Option<ScanVisits>)>> {
        let mut store = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let mut roots = BTreeMap::<String, (String, u64, bool, Option<ScanVisits>)>::new();
        let mut add = |record: Record| {
            let normalized = key(&record.path);
            if let Some(visits) = record.visits {
                roots
                    .entry(normalized)
                    .or_insert((record.path, 0, false, None))
                    .3 = Some(visits);
                return;
            }
            match record.size {
                None => {
                    roots
                        .entry(normalized)
                        .or_insert((record.path, 0, false, None))
                        .2 = true;
                }
                Some(size) => {
                    roots
                        .entry(normalized)
                        .or_insert((record.path, 0, false, None))
                        .1 = size;
                }
            }
        };
        // Disk records precede the in-memory fallback tail.
        if let Some(spill) = &mut store.spill {
            let file = &mut spill.file;
            file.seek(SeekFrom::Start(0))?;
            let end = file.metadata()?.len();
            while file.stream_position()? < end {
                add(read_record(file, end)?);
            }
        }
        for record in &store.memory {
            add(Record {
                path: record.path.clone(),
                size: record.size,
                visits: record.visits,
            });
        }
        Ok(roots)
    }
}
fn private_spill(records: &[Record]) -> io::Result<Spill> {
    let directory = tempfile::tempdir()?;
    let pin = crate::local_access::DirectoryHandle::open_root(directory.path())?;
    pin.secure_private()?;
    let name = std::ffi::OsStr::new("retry-roots");
    drop(pin.create_file_new(name)?);
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join(name))?;
    // No private path bytes precede the owner-mode / protected-DACL check.
    crate::local_access::secure_private_handle(&file, false)?;
    for record in records {
        write_record(&mut file, record)?;
    }
    Ok(Spill {
        file,
        _pin: pin,
        _directory: directory,
    })
}
fn write_record(file: &mut File, record: &Record) -> io::Result<()> {
    let end = file.seek(SeekFrom::End(0))?;
    let result = (|| {
        file.write_all(&[if record.visits.is_some() {
            2
        } else {
            u8::from(record.size.is_some())
        }])?;
        file.write_all(&(record.path.len() as u64).to_le_bytes())?;
        file.write_all(record.path.as_bytes())?;
        if let Some(size) = record.size {
            file.write_all(&size.to_le_bytes())?;
        }
        if let Some(visits) = record.visits {
            file.write_all(&visits.scanned.to_le_bytes())?;
            file.write_all(&visits.bytes.to_le_bytes())?;
        }
        Ok(())
    })();
    if result.is_err() {
        file.set_len(end)?;
    }
    result
}
fn read_record(file: &mut File, end: u64) -> io::Result<Record> {
    let mut tag = [0];
    let mut size = [0; 8];
    file.read_exact(&mut tag)?;
    file.read_exact(&mut size)?;
    let length = u64::from_le_bytes(size);
    if tag[0] > 2 || length > end.saturating_sub(file.stream_position()?) {
        return Err(io::Error::other("incomplete retry-root record"));
    }
    let length = usize::try_from(length).map_err(io::Error::other)?;
    let mut bytes = vec![0; length];
    file.read_exact(&mut bytes)?;
    let subtotal = if tag[0] == 1 {
        file.read_exact(&mut size)?;
        Some(u64::from_le_bytes(size))
    } else {
        None
    };
    let visits = if tag[0] == 2 {
        file.read_exact(&mut size)?;
        let scanned = u64::from_le_bytes(size);
        file.read_exact(&mut size)?;
        Some(ScanVisits {
            scanned,
            bytes: u64::from_le_bytes(size),
        })
    } else {
        None
    };
    Ok(Record {
        path: String::from_utf8(bytes).map_err(io::Error::other)?,
        size: subtotal,
        visits,
    })
}
