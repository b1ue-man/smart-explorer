//! Private immutable intent records; capture waits for their durable write.
use super::{
    platform::{self, StoreArea},
    record::{self, Record},
};
use std::{ffi::OsStr, fs::File, io, path::Path};

pub(super) struct Store {
    pub(super) area: StoreArea,
    _lock: File,
}
impl Store {
    pub(super) fn open() -> io::Result<Self> {
        Self::at(&crate::support_dirs::app_data_dir().join("host-trash"))
    }
    pub(super) fn at(path: &Path) -> io::Result<Self> {
        let area = platform::open_area(path)?;
        let lock = platform::lock(&area)?;
        Ok(Self { area, _lock: lock })
    }
    pub(super) fn persist(&self, record: &Record) -> io::Result<()> {
        record.validate()?;
        let bytes = serde_json::to_vec(record).map_err(io::Error::other)?;
        if bytes.len() as u64 > record::MAX_RECORD_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Papierkorb-Intent überschreitet die Pfadgrenze",
            ));
        }
        let name = format!("{}.json", record.id);
        // Reserve the ID exclusively before using the established private
        // atomic writer. An interrupted reservation is visible but cannot
        // have captured content; no incomplete record authorizes a rename.
        drop(self.area.directory.create_file_new(OsStr::new(&name))?);
        // This facade syncs the private stage and uses V1's write-through
        // promotion on Windows. A no-op directory sync alone is insufficient.
        crate::support_dirs::write_private_atomic(&self.area.path.join(name), &bytes)?;
        Ok(())
    }
    pub(super) fn load(&self, id: &str) -> io::Result<Record> {
        if !record::lower_hex(id, 32) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Ungültiger Papierkorb-Eintrag",
            ));
        }
        let path = self.area.path.join(format!("{id}.json"));
        let raw = crate::support_dirs::read_private_text(&path, record::MAX_RECORD_BYTES)?;
        let record: Record = serde_json::from_str(&raw).map_err(io::Error::other)?;
        record.validate()?;
        if record.id != id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Papierkorb-Intent gehört zu einer anderen ID",
            ));
        }
        Ok(record)
    }
}
