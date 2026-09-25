//! Small JSON files of the facade (`<data>/mobile/*.json`), replaced
//! atomically so a killed process never leaves a half-written file.
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Files larger than this are treated as damaged.
const MAX_STORE_BYTES: u64 = 8 * 1024 * 1024;

/// Reads `path`; a missing file is `T::default()`.
pub(crate) fn read_json<T: DeserializeOwned + Default>(path: &Path) -> io::Result<T> {
    let bytes = match std::fs::metadata(path) {
        Ok(metadata) if metadata.len() > MAX_STORE_BYTES => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} ist zu groß", path.display()),
            ))
        }
        Ok(_) => std::fs::read(path)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(T::default()),
        Err(error) => return Err(error),
    };
    serde_json::from_slice(&bytes).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: {error}", path.display()),
        )
    })
}

/// Writes `value` to a sibling temp file, syncs it and renames it over `path`.
pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    let temp = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
