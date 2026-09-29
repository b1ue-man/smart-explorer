//! All variants are recoverable before the first replacement or exact-ID trash.
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use crate::vfs::Backend;
use super::duplicate_observation::{read_content, verify_content};
use super::duplicate_types::FileVariant;

pub(super) fn save(
    backend: &dyn Backend, path: &str, rel: &str, versions: &Path,
    variant: &FileVariant, cancel: &AtomicBool, throttle: &super::types::Throttle,
) -> io::Result<()> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs()).unwrap_or(0);
    for offset in 0..1000 {
        let target = versions.join(stamp.saturating_add(offset).to_string()).join(rel);
        if let Some(parent) = target.parent() { std::fs::create_dir_all(parent)?; }
        let mut file = match std::fs::OpenOptions::new().write(true).create_new(true).open(&target) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let saved = (|| {
            verify_content(read_content(backend, path, variant.id.as_deref(), &mut file, cancel, Some(throttle))?, variant)?;
            file.flush()?;
            file.sync_all()
        })();
        drop(file);
        if let Err(error) = saved {
            let _ = std::fs::remove_file(target);
            return Err(error);
        }
        return Ok(());
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "Kein freier Platz für eine Wiederherstellungsversion"))
}
