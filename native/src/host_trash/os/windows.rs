//! Ordinary Windows handles, original-path recovery and private store locking.
use std::{ffi::{OsStr, OsString}, fs::File, io::{self, Read, Seek, SeekFrom},
    mem::size_of, path::{Component, Path, PathBuf}};
use std::os::windows::{ffi::{OsStrExt, OsStringExt}, fs::OpenOptionsExt, io::AsRawHandle};
use sha2::{Digest, Sha256};
use windows_sys::Win32::{Foundation::{GENERIC_READ, GENERIC_WRITE, ERROR_SHARING_VIOLATION},
    Storage::FileSystem::{FileIdInfo, GetFileInformationByHandleEx, GetFinalPathNameByHandleW,
        FILE_ID_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_READ_ATTRIBUTES, FILE_SHARE_READ, READ_CONTROL, WRITE_DAC}};
use crate::{local_access::DirectoryHandle, vfs::{RecycleExpectation, RecycleOutcome}};
use super::{record::{self, FileIdentity, Record}, store::Store, CatalogPage, RestoreOutcome};

pub(crate) fn available() -> bool { true }
pub(crate) fn list(cursor: Option<&str>) -> io::Result<CatalogPage> { super::catalog::list(cursor) }
pub(crate) fn restore(id: &str) -> io::Result<RestoreOutcome> { super::restore::restore(id) }

pub(super) struct StoreArea { pub(super) path: PathBuf, pub(super) directory: DirectoryHandle }
pub(super) fn open_area(path: &Path) -> io::Result<StoreArea> {
    crate::support_dirs::ensure_private_dir(path)?;
    let directory = DirectoryHandle::open_root(path)?;
    directory.secure_private()?;
    let path = std::fs::canonicalize(path)?;
    Ok(StoreArea { path, directory })
}
pub(super) fn lock(area: &StoreArea) -> io::Result<File> {
    match area.directory.create_file_new(OsStr::new("operation.lock")) {
        Ok(file) => { file.sync_all()?; }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    // Only one writer can own this reservation, across GUI/daemon processes.
    let file = std::fs::OpenOptions::new()
        .access_mode(GENERIC_READ | GENERIC_WRITE | FILE_READ_ATTRIBUTES | READ_CONTROL | WRITE_DAC)
        .share_mode(FILE_SHARE_READ).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(area.path.join("operation.lock")).map_err(|error| {
            if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION as i32) {
                io::Error::new(io::ErrorKind::WouldBlock, "Papierkorb wird gerade bearbeitet; bitte erneut versuchen")
            } else { error }
        })?;
    crate::local_access::secure_private_handle(&file, false)?;
    Ok(file)
}

pub(super) fn identity(file: &File) -> io::Result<FileIdentity> {
    let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: live handle and correctly sized, aligned integer output record.
    let ok = unsafe { GetFileInformationByHandleEx(file.as_raw_handle(), FileIdInfo,
        (&mut info as *mut FILE_ID_INFO).cast(), size_of::<FILE_ID_INFO>() as u32) };
    if ok == 0 { return Err(io::Error::last_os_error()); }
    if info.VolumeSerialNumber == 0 || info.FileId.Identifier == [0; 16] {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "Dateisystem meldet keine sichere Dateiidentität"));
    }
    Ok(FileIdentity { volume: info.VolumeSerialNumber, file: info.FileId.Identifier })
}

fn final_path(file: &File) -> io::Result<PathBuf> {
    let mut wide = vec![0u16; record::MAX_PATH_UNITS + 1];
    // SAFETY: live handle and writable TCHAR buffer; zero flags select the
    // normalized DOS/UNC path. No free source path is resolved again.
    let length = unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(), wide.as_mut_ptr(), wide.len() as u32, 0) };
    if length == 0 { return Err(io::Error::last_os_error()); }
    if length as usize >= wide.len() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Originalpfad überschreitet die Windows-Pfadgrenze"));
    }
    wide.truncate(length as usize);
    let path = PathBuf::from(OsString::from_wide(&wide));
    if !path.is_absolute() { return Err(io::Error::new(io::ErrorKind::InvalidData, "Originalpfad ist nicht absolut")); }
    Ok(path)
}
fn root_probe(root: &Path) -> io::Result<File> {
    let file = std::fs::OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT).open(root)?;
    let class = crate::local_access::classify_open_file(&file)?;
    if class.link_like || class.special || !file.metadata()?.is_dir() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Aufgezeichnete Wurzel wurde umgeleitet"));
    }
    Ok(file)
}

pub(super) fn digest(file: &File, size: u64) -> io::Result<String> {
    let before = file.metadata()?;
    if before.len() != size { return Err(io::Error::new(io::ErrorKind::InvalidData, "Dateilänge wurde verändert")); }
    let mut read = file.try_clone()?;
    read.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new(); let mut bytes = 0u64; let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let n = match read.read(&mut buffer) { Err(error) if error.kind() == io::ErrorKind::Interrupted => continue, result => result? };
        if n == 0 { break; }
        bytes = bytes.checked_add(n as u64).ok_or_else(|| io::Error::other("Dateilänge erschöpft"))?;
        if bytes > size { return Err(io::Error::new(io::ErrorKind::InvalidData, "Datei wurde während der Prüfung verändert")); }
        hash.update(&buffer[..n]);
    }
    let after = file.metadata()?;
    if bytes != size || before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Datei wurde während der Prüfung verändert"));
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub(super) fn verify(file: &File, record: &Record) -> io::Result<()> {
    if identity(file)? != record.file_identity || digest(file, record.size)? != record.sha256 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Aufbewahrter Inhalt wurde verändert"));
    }
    Ok(())
}

fn intent(root: &Path, file: &File, size: u64, sha256: String) -> io::Result<Record> {
    let probe = root_probe(root)?;
    let root = final_path(&probe)?;
    let original = final_path(file)?;
    let relative = original.strip_prefix(&root).map_err(|_| io::Error::new(io::ErrorKind::PermissionDenied,
        "Geprüftes Objekt liegt außerhalb der aufgezeichneten Wurzel"))?;
    let relative = relative.components().map(|component| match component {
        Component::Normal(name) => Ok(name.encode_wide().collect()),
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Kein regulärer Child-Pfad")),
    }).collect::<io::Result<Vec<Vec<u16>>>>()?;
    let mut nonce = [0u8; 32];
    getrandom::getrandom(&mut nonce).map_err(|error| io::Error::other(error.to_string()))?;
    let hex = |bytes: &[u8]| bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    let created_ms = chrono::Utc::now().timestamp_millis();
    let record = Record { version: record::VERSION,
        id: format!("{:016x}{}", created_ms.max(0) as u64, hex(&nonce[8..16])),
        created_ms, root: root.as_os_str().encode_wide().collect(),
        relative, root_identity: identity(&probe)?, file_identity: identity(file)?, size, sha256,
        held: format!(".held.se-recycle-{}", hex(&nonce[..8])),
        restore_held: format!(".held.se-recycle-{}", hex(&nonce[24..])) };
    record.validate()?;
    Ok(record)
}

pub(crate) fn recycle_selected(root: &Path, parent: &DirectoryHandle,
    file: &File, expected: &RecycleExpectation,
) -> io::Result<RecycleOutcome> {
    if expected.sha256.as_ref().is_some_and(|hash| !record::lower_hex(hash, 64)) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Ungültige erwartete SHA-256"));
    }
    if file.metadata()?.len() != expected.size { return Ok(RecycleOutcome::Changed); }
    let hash = digest(file, expected.size)?;
    if expected.sha256.as_ref().is_some_and(|expected| expected != &hash) { return Ok(RecycleOutcome::Changed); }
    let record = intent(root, file, expected.size, hash)?;
    let store = Store::open()?;
    capture(&store, &record, parent, file)
}

pub(super) fn capture(store: &Store, record: &Record, parent: &DirectoryHandle, file: &File) -> io::Result<RecycleOutcome> {
    record.validate()?;
    let original = original_name(record)?;
    let slot = DirectoryHandle::checked_quarantine_slot(OsStr::new(&record.held))?;
    // This complete immutable mapping (including restore hop) is durable
    // before the very first namespace change of the source.
    store.persist(record)?;
    let mut captured = parent.quarantine_regular_child_in(&original, file, &slot)?;
    if let Err(error) = verify(captured.file(), record) {
        if let Err(restore) = captured.restore() {
            return Err(io::Error::new(restore.kind(),
                format!("{error}; Wiederherstellen fehlgeschlagen: {restore}; Eintrag {} bleibt auffindbar", record.id)));
        }
        return Ok(RecycleOutcome::Changed);
    }
    // Drop retains the original-ACL payload. The durable record and exact
    // private name are its recovery route; no shell or permanent delete.
    Ok(RecycleOutcome::Recycled)
}

fn original_name(record: &Record) -> io::Result<OsString> {
    record.relative.last().map(|name| OsString::from_wide(name))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Fehlender Originalname"))
}
pub(super) fn original_path(record: &Record) -> PathBuf {
    let mut path = PathBuf::from(OsString::from_wide(&record.root));
    for name in &record.relative { path.push(OsString::from_wide(name)); }
    path
}
pub(super) fn open_parent(record: &Record) -> io::Result<DirectoryHandle> {
    record.validate()?;
    let root = PathBuf::from(OsString::from_wide(&record.root));
    if !root.is_absolute() { return Err(io::Error::new(io::ErrorKind::InvalidData, "Aufgezeichnete Wurzel ist relativ")); }
    let mut parent = DirectoryHandle::open_root(&root)?;
    let probe = root_probe(&root)?;
    if identity(&probe)? != record.root_identity {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Aufgezeichnete Wurzel wurde ersetzt"));
    }
    for name in &record.relative[..record.relative.len()-1] { parent = parent.open_child(&OsString::from_wide(name))?; }
    Ok(parent)
}

pub(super) struct HeldFile {
    pub(super) parent: DirectoryHandle, pub(super) file: File, pub(super) slot: usize,
    pub(super) original: OsString,
}
pub(super) enum Location { Held(HeldFile), Original, Missing }

pub(super) fn locate(record: &Record) -> io::Result<Location> {
    let parent = open_parent(record)?;
    let mut found = None; let mut foreign = false; let mut problem = None;
    for (slot, name) in record.slots().into_iter().enumerate() {
        match parent.open_regular_child(OsStr::new(name)) {
            Ok(file) => {
                if identity(&file)? == record.file_identity {
                    if file.metadata()?.len() != record.size { return Err(io::Error::new(io::ErrorKind::InvalidData, "Aufbewahrte Dateilänge wurde verändert")); }
                    if found.is_some() { return Err(io::Error::new(io::ErrorKind::InvalidData, "Mehrdeutige aufbewahrte Dateiidentität")); }
                    found = Some(HeldFile { parent:parent.clone(), file, slot, original:original_name(record)? });
                } else { foreign = true; }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => { problem = Some(error); }
        }
    }
    if let Some(found) = found { return Ok(Location::Held(found)); }
    match parent.open_regular_child(&original_name(record)?) {
        Ok(file) if identity(&file)? == record.file_identity => return Ok(Location::Original),
        Ok(_) => foreign = true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => problem = Some(error),
    }
    if let Some(error) = problem { return Err(error); }
    if foreign { return Err(io::Error::new(io::ErrorKind::InvalidData, "Die aufgezeichneten Positionen enthalten ein anderes Objekt")); }
    Ok(Location::Missing)
}

#[cfg(test)]
pub(super) fn test_intent(root: &Path, file: &File, size: u64) -> io::Result<Record> {
    intent(root, file, size, digest(file, size)?)
}

#[cfg(test)]
pub(super) fn test_symlink_file(original: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(original, link)
}
