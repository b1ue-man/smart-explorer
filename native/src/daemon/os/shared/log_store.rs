//! Cross-process append and bounded rotation; no diagnostic history is
//! truncated in place. The same OS file lock also serializes job state.
use std::io::{self, Write};
use std::path::Path;
const CAP: u64 = 256 * 1024;
const ARCHIVES: usize = 3;
pub(super) fn log(message: &str) {
    let directory = crate::support_dirs::sync_data_dir();
    let result = (|| -> io::Result<()> {
        std::fs::create_dir_all(&directory)?;
        let _lock = crate::syncjobs::RuntimeFileLock::acquire(&directory.join("daemon.log.lock"), std::time::Duration::from_millis(500))?;
        append(&directory, message)
    })();
    if let Err(error) = result { eprintln!("Smart Explorer worker log: {error}; {message}"); }
}
fn append(directory: &Path, message: &str) -> io::Result<()> {
    let path = directory.join("daemon.log");
    if let Ok(metadata) = std::fs::symlink_metadata(&path) {
        if !metadata.file_type().is_file() { return Err(io::Error::new(io::ErrorKind::PermissionDenied, "log must be a regular file")); }
        if metadata.len() >= CAP {
            remove_optional(&directory.join(format!("daemon.log.{ARCHIVES}")))?;
            for number in (1..ARCHIVES).rev() {
                rename_optional(&directory.join(format!("daemon.log.{number}")), &directory.join(format!("daemon.log.{}", number + 1)))?;
            }
            std::fs::rename(&path, directory.join("daemon.log.1"))?;
        }
    }
    let mut end = message.len().min(8192);
    while !message.is_char_boundary(end) { end -= 1; }
    let message = message[..end].replace(['\r', '\n'], " ");
    let mut file = super::platform::open_log(&path)?;
    writeln!(file, "{} {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), message)
}
fn remove_optional(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) { Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()), result => result }
}
fn rename_optional(source: &Path, target: &Path) -> io::Result<()> {
    match std::fs::rename(source, target) { Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()), result => result }
}
